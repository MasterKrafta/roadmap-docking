use std::collections::{HashMap, HashSet};

use super::{
    WindowGrid, WindowGridDirection, WindowGridLink, WindowGridMember, WindowGridPruneResult,
};

/// Registry of all linked Floating window components.
///
/// Standalone windows are intentionally implicit and are returned as
/// one-member results from `members_for`. This mirrors the previous
/// WindowChainRegistry behavior while allowing each component to hold 2D links.
#[derive(Clone, Debug, Default)]
pub(crate) struct WindowGridRegistry {
    grids: HashMap<WindowGridMember, WindowGrid>,
    grid_by_group: HashMap<WindowGridMember, WindowGridMember>,
}

impl WindowGridRegistry {
    /// Returns every directed edge once, including all currently connected grids.
    pub(crate) fn all_links(&self) -> Vec<WindowGridLink> {
        let mut emitted_grids = HashSet::new();
        let mut links = Vec::new();
        for grid_id in self.grid_by_group.values() {
            if !emitted_grids.insert(*grid_id) {
                continue;
            }
            if let Some(grid) = self.grids.get(grid_id) {
                links.extend(grid.links.iter().copied());
            }
        }
        links
    }

    /// Rebuilds the registry transactionally from a complete set of directed edges.
    ///
    /// Each input edge must connect two distinct components at the point it is
    /// inserted; cycles and duplicate identities are rejected without changing
    /// the current registry.
    pub(crate) fn replace_all_links(&mut self, links: &[WindowGridLink]) -> bool {
        let mut next = Self::default();
        let mut seen = HashSet::new();

        for link in links {
            if !seen.insert(*link) {
                continue;
            }
            if !next.link_to_side(link.from, link.to, link.direction) {
                return false;
            }
        }

        if !next.is_valid() {
            return false;
        }
        *self = next;
        true
    }

    pub(crate) fn members_for(&self, group: WindowGridMember) -> Vec<WindowGridMember> {
        self.grid_by_group
            .get(&group)
            .and_then(|grid_id| self.grids.get(grid_id))
            .map_or_else(|| vec![group], |grid| grid.members.clone())
    }

    pub(crate) fn is_chained(&self, group: WindowGridMember) -> bool {
        self.grid_by_group.contains_key(&group)
    }

    /// Keeps the existing left/right insertion API for current callers.
    pub(crate) fn link_before(
        &mut self,
        anchor: WindowGridMember,
        moving: WindowGridMember,
    ) -> bool {
        self.link_to_side(anchor, moving, WindowGridDirection::Left)
    }

    /// Keeps the existing left/right insertion API for current callers.
    pub(crate) fn link_after(
        &mut self,
        anchor: WindowGridMember,
        moving: WindowGridMember,
    ) -> bool {
        self.link_to_side(anchor, moving, WindowGridDirection::Right)
    }

    /// Joins the moving component on the requested side of the anchor.
    ///
    /// Existing internal links and member order are retained. Multiple neighbours
    /// on one side are represented as separate branches; a new window is not
    /// silently inserted between existing neighbours. Joining members already
    /// in one component is rejected because it would create a cycle or an
    /// ambiguous second route through the same component.
    #[allow(dead_code)] // Consumed by the four-direction Drop stage.
    pub(crate) fn link_to_side(
        &mut self,
        anchor: WindowGridMember,
        moving: WindowGridMember,
        side: WindowGridDirection,
    ) -> bool {
        if anchor == moving {
            return false;
        }

        let anchor_grid_id = self.grid_by_group.get(&anchor).copied();
        let moving_grid_id = self.grid_by_group.get(&moving).copied();
        if anchor_grid_id.is_some() && anchor_grid_id == moving_grid_id {
            return false;
        }

        let mut next = self.clone();
        let anchor_grid = next
            .take_grid_for(anchor)
            .unwrap_or_else(|| WindowGrid::single(anchor));
        let moving_grid = next
            .take_grid_for(moving)
            .unwrap_or_else(|| WindowGrid::single(moving));

        let Some(anchor_index) = anchor_grid
            .members
            .iter()
            .position(|member| *member == anchor)
        else {
            return false;
        };

        // Repeated drops on an occupied side extend that side outward in a
        // deterministic lane instead of placing several HWNDs at the same
        // coordinates. Left/Above lanes are ordered from farthest to nearest;
        // Right/Below lanes are ordered from nearest to farthest.
        let side_member_indices = anchor_grid
            .members
            .iter()
            .enumerate()
            .filter_map(|(index, member)| {
                (anchor_grid.direction_between(anchor, *member) == Some(side))
                    .then_some(index)
            })
            .collect::<Vec<_>>();

        let insertion_index = match side {
            WindowGridDirection::Left | WindowGridDirection::Above => {
                side_member_indices
                    .into_iter()
                    .min()
                    .unwrap_or(anchor_index)
            }
            WindowGridDirection::Right | WindowGridDirection::Below => {
                side_member_indices
                    .into_iter()
                    .max()
                    .map_or(anchor_index + 1, |index| index + 1)
            }
        };

        let Some(moving_boundary) =
            moving_grid.boundary_member(side.opposite(), moving)
        else {
            return false;
        };

        let mut members = anchor_grid.members;
        members.splice(insertion_index..insertion_index, moving_grid.members);

        let mut links = anchor_grid.links;
        links.extend(moving_grid.links);
        let Some(link) = WindowGridLink::new(anchor, moving_boundary, side) else {
            return false;
        };
        links.push(link);

        let grid = WindowGrid { members, links };
        if !grid.is_valid() {
            return false;
        }

        next.store_grid(grid);
        if !next.is_valid() {
            return false;
        }

        *self = next;
        true
    }

    /// Joins an ordered horizontal row of newly-created windows to one side
    /// of an anchor. The row's internal left-to-right order is preserved.
    ///
    /// The complete operation is transactional so a failed link leaves both
    /// the source sequence and destination grid unchanged.
    pub(crate) fn link_sequence_to_side(
        &mut self,
        anchor: WindowGridMember,
        moving: &[WindowGridMember],
        side: WindowGridDirection,
    ) -> bool {
        if moving.is_empty() {
            return false;
        }
        let mut seen = HashSet::new();
        if moving
            .iter()
            .any(|group| *group == anchor || !seen.insert(*group))
        {
            return false;
        }

        let mut next = self.clone();
        for pair in moving.windows(2) {
            if !next.link_after(pair[0], pair[1]) {
                return false;
            }
        }

        if !next.link_to_side(anchor, moving[0], side) || !next.is_valid() {
            return false;
        }

        *self = next;
        true
    }

    pub(crate) fn remove_member(&mut self, group: WindowGridMember) -> bool {
        let Some(mut grid) = self.take_grid_for(group) else {
            return false;
        };
        grid.remove_member(group);
        for component in grid.connected_components() {
            self.store_grid(component);
        }
        debug_assert!(self.is_valid());
        true
    }

    /// Result of pruning stale FloatingGroups during reconciliation.
    ///
    /// Straight-through candidates are deliberately not linked yet: the manager
    /// must validate the proposed geometry against the surviving native windows.
    pub(crate) fn retain_members(
        &mut self,
        live_members: &HashSet<WindowGridMember>,
    ) -> WindowGridPruneResult {
        let previous_grids = std::mem::take(&mut self.grids)
            .into_values()
            .collect::<Vec<_>>();
        self.grid_by_group.clear();

        let mut result = WindowGridPruneResult::default();

        for mut grid in previous_grids {
            let stale_members = grid
                .members
                .iter()
                .copied()
                .filter(|member| !live_members.contains(member))
                .collect::<Vec<_>>();
            result.removed_members += stale_members.len();

            let had_stale_members = !stale_members.is_empty();
            let mut temporary_reconnections = Vec::new();
            for stale in &stale_members {
                if let Some(candidate) = grid.remove_member_inner(*stale, true) {
                    temporary_reconnections.push(candidate);
                }
            }

            // The temporary links let consecutive stale nodes collapse into
            // one straight-through candidate, but none is published before
            // native geometry validation in FloatingWindowManager.
            for candidate in temporary_reconnections {
                grid.links.retain(|link| *link != candidate);
                if live_members.contains(&candidate.from)
                    && live_members.contains(&candidate.to)
                    && !result.reconnect_candidates.contains(&candidate)
                {
                    result.reconnect_candidates.push(candidate);
                }
            }

            for component in grid.connected_components() {
                if had_stale_members && component.members.len() > 1 {
                    result.components_to_align.push(component.members[0]);
                }
                self.store_grid(component);
            }
        }

        debug_assert!(self.is_valid());
        result
    }

    /// Provides a stable snapshot of a component's explicit adjacency links.
    #[allow(dead_code)] // Consumed by geometry, Z-order, and lifecycle stages.
    pub(crate) fn links_for(
        &self,
        group: WindowGridMember,
    ) -> Option<&[WindowGridLink]> {
        let grid_id = self.grid_by_group.get(&group)?;
        self.grids.get(grid_id).map(|grid| grid.links.as_slice())
    }

    /// Returns the relative side of `to` from `from`, if they are linked.
    #[allow(dead_code)] // Used by the geometry and four-direction Drop stages.
    pub(crate) fn direction_between(
        &self,
        from: WindowGridMember,
        to: WindowGridMember,
    ) -> Option<WindowGridDirection> {
        let grid_id = self.grid_by_group.get(&from)?;
        self.grids.get(grid_id)?.direction_between(from, to)
    }

    /// Returns whether this window has a direct neighbour on the requested side.
    ///
    /// Resize-edge ownership is local to each node: a window may have a west
    /// neighbour even when it is not the first item in the grid's stable order.
    pub(crate) fn has_neighbor_on_side(
        &self,
        group: WindowGridMember,
        side: WindowGridDirection,
    ) -> bool {
        self.links_for(group).is_some_and(|links| {
            links.iter().any(|link| {
                if link.from == group {
                    link.direction == side
                } else if link.to == group {
                    link.direction.opposite() == side
                } else {
                    false
                }
            })
        })
    }

    pub(super) fn is_valid(&self) -> bool {
        let mut indexed_members = HashSet::new();

        for (grid_id, grid) in &self.grids {
            if grid.members.first() != Some(grid_id) || !grid.is_valid() {
                return false;
            }

            for member in &grid.members {
                if !indexed_members.insert(*member)
                    || self.grid_by_group.get(member) != Some(grid_id)
                {
                    return false;
                }
            }
        }

        indexed_members.len() == self.grid_by_group.len()
            && self
                .grid_by_group
                .keys()
                .all(|member| indexed_members.contains(member))
    }

    fn take_grid_for(&mut self, group: WindowGridMember) -> Option<WindowGrid> {
        let grid_id = self.grid_by_group.get(&group).copied()?;
        let grid = self.grids.remove(&grid_id)?;
        for member in &grid.members {
            self.grid_by_group.remove(member);
        }
        Some(grid)
    }

    fn store_grid(&mut self, grid: WindowGrid) {
        if !grid.is_valid() {
            return;
        }

        let grid_id = grid.members[0];
        for member in &grid.members {
            self.grid_by_group.insert(*member, grid_id);
        }
        self.grids.insert(grid_id, grid);
    }
}
