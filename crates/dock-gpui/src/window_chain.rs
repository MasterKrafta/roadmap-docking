//! Logical membership and adjacency for independent native Floating windows.
//!
//! A WindowGrid stores only relationships between windows. It does not own
//! native handles, window bounds, or Core layout state.

use std::collections::{HashMap, HashSet, VecDeque};

use dock_core::{FloatingGroupId, NodeId};

/// Stable identity of one persistent native member in a Floating WindowGrid.
///
/// FloatingComposition is a compatibility bridge for the current one-window-per-
/// composition implementation. New Floating hosts will use Header and Group, so
/// a Core FloatingGroupId is no longer overloaded as the identity of every HWND.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(dead_code)] // Header and Group variants are consumed by the dedicated-window stage.
pub(crate) enum WindowGridMember {
    FloatingComposition(FloatingGroupId),
    Header(NodeId),
    Group(NodeId),
}

impl WindowGridMember {
    pub(crate) const fn floating_composition(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }

    /// Returns the legacy Core FloatingGroup represented by this member.
    /// Header and Group members will resolve through the dedicated host registry.
    pub(crate) const fn floating_group(self) -> Option<FloatingGroupId> {
        match self {
            Self::FloatingComposition(group) => Some(group),
            Self::Header(_) | Self::Group(_) => None,
        }
    }
}

impl From<FloatingGroupId> for WindowGridMember {
    fn from(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }
}

/// The side occupied by the target member relative to the source member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowGridDirection {
    Left,
    Right,
    Above,
    Below,
}

impl WindowGridDirection {
    pub(crate) fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Above => Self::Below,
            Self::Below => Self::Above,
        }
    }
}

/// One directed description of an undirected physical adjacency.
///
/// The reverse relationship is derived with `WindowGridDirection::opposite`;
/// the registry never stores both directions of the same connection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WindowGridLink {
    pub(crate) from: WindowGridMember,
    pub(crate) to: WindowGridMember,
    pub(crate) direction: WindowGridDirection,
}

impl WindowGridLink {
    fn new(
        from: WindowGridMember,
        to: WindowGridMember,
        direction: WindowGridDirection,
    ) -> Option<Self> {
        (from != to).then_some(Self {
            from,
            to,
            direction,
        })
    }
}

/// Connected component of independent Floating windows.
///
/// `members` preserves a deterministic traversal/insertion order for existing
/// move and native Z-order operations. `links` carries the actual 2D topology.
#[derive(Clone, Debug, Default)]
struct WindowGrid {
    members: Vec<WindowGridMember>,
    links: Vec<WindowGridLink>,
}

impl WindowGrid {
    fn single(group: WindowGridMember) -> Self {
        Self {
            members: vec![group],
            links: Vec::new(),
        }
    }

    fn direction_between(
        &self,
        from: WindowGridMember,
        to: WindowGridMember,
    ) -> Option<WindowGridDirection> {
        self.links.iter().find_map(|link| {
            if link.from == from && link.to == to {
                Some(link.direction)
            } else if link.from == to && link.to == from {
                Some(link.direction.opposite())
            } else {
                None
            }
        })
    }

    /// Chooses the member on the side of this component that faces the target.
    ///
    /// When a grid has a single exposed member on that edge, it is used even
    /// when a different member initiated the drag. For a multi-contact edge,
    /// prefer the grabbed member if it is itself on that boundary; otherwise
    /// the placement is ambiguous and must not be committed.
    fn boundary_member(
        &self,
        outward_side: WindowGridDirection,
        preferred: WindowGridMember,
    ) -> Option<WindowGridMember> {
        let candidates = self
            .members
            .iter()
            .copied()
            .filter(|member| {
                !self.links.iter().any(|link| {
                    if link.from == *member {
                        link.direction == outward_side
                    } else if link.to == *member {
                        link.direction.opposite() == outward_side
                    } else {
                        false
                    }
                })
            })
            .collect::<Vec<_>>();

        if candidates.contains(&preferred) {
            Some(preferred)
        } else if candidates.len() == 1 {
            candidates.first().copied()
        } else {
            None
        }
    }

    /// Removes a member and returns a possible straight-through replacement link.
    ///
    /// Reconnection is optional because a replacement edge must be checked
    /// against the surviving windows' native geometry before it is committed.
    fn remove_member_inner(
        &mut self,
        group: WindowGridMember,
        reconnect: bool,
    ) -> Option<WindowGridLink> {
        if !self.members.contains(&group) {
            return None;
        }

        let incident = self
            .links
            .iter()
            .filter_map(|link| {
                if link.from == group {
                    Some((link.to, link.direction))
                } else if link.to == group {
                    Some((link.from, link.direction.opposite()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let replacement = if let [(first, first_side), (second, second_side)] =
            incident.as_slice()
            && first_side.opposite() == *second_side
        {
            match first_side {
                WindowGridDirection::Left => {
                    WindowGridLink::new(*first, *second, WindowGridDirection::Right)
                }
                WindowGridDirection::Above => {
                    WindowGridLink::new(*first, *second, WindowGridDirection::Below)
                }
                WindowGridDirection::Right => {
                    WindowGridLink::new(*second, *first, WindowGridDirection::Right)
                }
                WindowGridDirection::Below => {
                    WindowGridLink::new(*second, *first, WindowGridDirection::Below)
                }
            }
        } else {
            None
        };

        self.members.retain(|member| *member != group);
        self.links
            .retain(|link| link.from != group && link.to != group);

        if reconnect
            && let Some(replacement) = replacement
            && self.direction_between(replacement.from, replacement.to).is_none()
        {
            self.links.push(replacement);
        }

        replacement
    }

    fn remove_member(&mut self, group: WindowGridMember) -> bool {
        if !self.members.contains(&group) {
            return false;
        }
        // Direct detach is fail-safe: reconnect only after the manager validates
        // the resulting native geometry through a prune candidate.
        self.remove_member_inner(group, false);
        true
    }

    /// Splits a graph after removal into its still-connected components.
    fn connected_components(&self) -> Vec<Self> {
        let mut visited = HashSet::new();
        let mut components = Vec::new();

        for start in &self.members {
            if !visited.insert(*start) {
                continue;
            }

            let mut component_members = HashSet::from([*start]);
            let mut pending = VecDeque::from([*start]);
            while let Some(current) = pending.pop_front() {
                for link in &self.links {
                    let neighbour = if link.from == current {
                        Some(link.to)
                    } else if link.to == current {
                        Some(link.from)
                    } else {
                        None
                    };

                    if let Some(neighbour) = neighbour
                        && visited.insert(neighbour)
                    {
                        component_members.insert(neighbour);
                        pending.push_back(neighbour);
                    }
                }
            }

            let members = self
                .members
                .iter()
                .copied()
                .filter(|member| component_members.contains(member))
                .collect::<Vec<_>>();
            let links = self
                .links
                .iter()
                .copied()
                .filter(|link| {
                    component_members.contains(&link.from)
                        && component_members.contains(&link.to)
                })
                .collect::<Vec<_>>();
            if !members.is_empty() {
                components.push(Self { members, links });
            }
        }

        components
    }

    /// Each stored grid is a connected, acyclic graph with unique member IDs.
    fn is_valid(&self) -> bool {
        if self.members.len() < 2 || self.links.len() != self.members.len() - 1 {
            return false;
        }

        let members = self.members.iter().copied().collect::<HashSet<_>>();
        if members.len() != self.members.len() {
            return false;
        }

        let mut edges = HashSet::new();
        for link in &self.links {
            if link.from == link.to
                || !members.contains(&link.from)
                || !members.contains(&link.to)
            {
                return false;
            }

            if edges.contains(&(link.to, link.from))
                || !edges.insert((link.from, link.to))
            {
                return false;
            }
        }

        self.connected_components().len() == 1
    }
}

/// Reconciliation details produced when stale WindowGrid members are pruned.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WindowGridPruneResult {
    pub(crate) removed_members: usize,
    pub(crate) reconnect_candidates: Vec<WindowGridLink>,
    pub(crate) components_to_align: Vec<WindowGridMember>,
}

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

    fn is_valid(&self) -> bool {
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

#[cfg(test)]
mod tests {
    use super::{WindowGridDirection, WindowGridMember, WindowGridRegistry};
    use dock_core::{FloatingGroupId, NodeId};
    use slotmap::SlotMap;
    use std::collections::HashSet;

    fn groups(count: usize) -> Vec<WindowGridMember> {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        (0..count)
            .map(|_| WindowGridMember::FloatingComposition(ids.insert(())))
            .collect()
    }

    #[test]
    fn header_group_and_legacy_composition_have_distinct_member_identities() {
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let header = WindowGridMember::Header(node_ids.insert(()));
        let group = WindowGridMember::Group(node_ids.insert(()));
        let composition = WindowGridMember::FloatingComposition(floating_ids.insert(()));
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(header, group));
        assert!(grid.link_after(group, composition));
        assert_eq!(grid.members_for(header), vec![header, group, composition]);
        assert_eq!(grid.direction_between(header, group), Some(WindowGridDirection::Right));
        assert_eq!(header.floating_group(), None);
        assert_eq!(group.floating_group(), None);
        assert!(composition.floating_group().is_some());
        assert!(grid.is_valid());
    }

    #[test]
    fn links_before_and_after_keep_the_existing_horizontal_order() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        assert!(grid.link_before(b, d));
        assert_eq!(grid.members_for(a), vec![d, a, b, c]);
        assert_eq!(grid.members_for(c), vec![d, a, b, c]);
        assert_eq!(grid.direction_between(b, d), Some(WindowGridDirection::Left));
    }

    #[test]
    fn supports_all_four_adjacency_directions_and_their_reverse() {
        let [left, right, above, below, anchor] =
            groups(5).try_into().expect("five group ids");

        for (moving, side, opposite) in [
            (left, WindowGridDirection::Left, WindowGridDirection::Right),
            (right, WindowGridDirection::Right, WindowGridDirection::Left),
            (above, WindowGridDirection::Above, WindowGridDirection::Below),
            (below, WindowGridDirection::Below, WindowGridDirection::Above),
        ] {
            let mut grid = WindowGridRegistry::default();
            assert!(grid.link_to_side(anchor, moving, side));
            assert_eq!(grid.direction_between(anchor, moving), Some(side));
            assert_eq!(grid.direction_between(moving, anchor), Some(opposite));
            assert!(grid.is_valid());
        }
    }

    #[test]
    fn joining_existing_components_preserves_their_internal_links() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_to_side(c, d, WindowGridDirection::Below));
        assert!(grid.link_to_side(a, c, WindowGridDirection::Below));

        assert_eq!(grid.members_for(a), vec![a, c, d, b]);
        assert_eq!(grid.direction_between(a, b), Some(WindowGridDirection::Right));
        assert_eq!(grid.direction_between(c, d), Some(WindowGridDirection::Below));
        assert_eq!(grid.direction_between(a, c), Some(WindowGridDirection::Below));
        assert!(grid.is_valid());
    }

    #[test]
    fn ambiguous_moving_grid_boundary_is_rejected_without_mutating_either_grid() {
        let [anchor, branch, moving, lower, other] =
            groups(5).try_into().expect("five group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_to_side(moving, branch, WindowGridDirection::Right));
        assert!(grid.link_to_side(moving, lower, WindowGridDirection::Below));
        let before_anchor = grid.members_for(anchor);
        let before_moving = grid.members_for(moving);
        let before_links = grid.links_for(moving).unwrap().to_vec();

        // The moving component has two exposed west-side candidates. Since the
        // grabbed member is not on that boundary, choosing a replacement edge
        // would be ambiguous and must leave both components untouched.
        assert!(!grid.link_to_side(anchor, branch, WindowGridDirection::Right));
        assert_eq!(grid.members_for(anchor), before_anchor);
        assert_eq!(grid.members_for(moving), before_moving);
        assert_eq!(grid.links_for(moving), Some(before_links.as_slice()));
        assert!(!grid.is_chained(other));
        assert!(grid.is_valid());
    }

    #[test]
    fn permits_multiple_windows_on_the_same_side_as_separate_branches() {
        for side in [
            WindowGridDirection::Left,
            WindowGridDirection::Right,
            WindowGridDirection::Above,
            WindowGridDirection::Below,
        ] {
            let [a, b, c] = groups(3).try_into().expect("three group ids");
            let mut grid = WindowGridRegistry::default();

            assert!(grid.link_to_side(a, b, side));
            assert!(grid.link_to_side(a, c, side));
            assert_eq!(grid.direction_between(a, b), Some(side));
            assert_eq!(grid.direction_between(a, c), Some(side));
            assert_eq!(
                grid.members_for(a),
                if matches!(side, WindowGridDirection::Left | WindowGridDirection::Above) {
                    vec![c, b, a]
                } else {
                    vec![a, b, c]
                },
                "same-side members are ordered from nearest to farthest in the physical lane",
            );
            assert!(grid.is_valid());
        }
    }

    #[test]
    fn rejects_self_links_duplicate_edges_and_cycles_without_mutating_state() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(!grid.link_to_side(a, a, WindowGridDirection::Right));
        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        let before = grid.clone();

        assert!(!grid.link_to_side(a, b, WindowGridDirection::Right));
        assert!(!grid.link_to_side(a, c, WindowGridDirection::Right));
        assert_eq!(grid.members_for(a), before.members_for(a));
        assert_eq!(grid.links_for(a), before.links_for(a));
        assert!(grid.is_valid());
    }

    #[test]
    fn sequence_join_preserves_horizontal_order_for_each_target_side() {
        let [anchor, a, b, c] = groups(4).try_into().expect("four group ids");

        for side in [
            WindowGridDirection::Left,
            WindowGridDirection::Right,
            WindowGridDirection::Above,
            WindowGridDirection::Below,
        ] {
            let mut grid = WindowGridRegistry::default();
            assert!(grid.link_sequence_to_side(anchor, &[a, b, c], side));
            assert_eq!(grid.direction_between(a, b), Some(WindowGridDirection::Right));
            assert_eq!(grid.direction_between(b, c), Some(WindowGridDirection::Right));
            let attached = if side == WindowGridDirection::Left { c } else { a };
            assert_eq!(grid.direction_between(anchor, attached), Some(side));
            assert!(grid.is_valid());
        }
    }

    #[test]
    fn a_sequence_cannot_be_joined_to_its_own_grid() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();
        assert!(grid.link_after(a, b));
        let before = grid.clone();

        assert!(!grid.link_sequence_to_side(a, &[b, c], WindowGridDirection::Below));
        assert_eq!(grid.members_for(a), before.members_for(a));
        assert_eq!(grid.links_for(a), before.links_for(a));
    }

    #[test]
    fn resize_edge_ownership_uses_direct_neighbours_not_member_order() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_to_side(a, b, WindowGridDirection::Right));
        assert!(grid.link_to_side(a, c, WindowGridDirection::Below));
        assert!(grid.link_to_side(c, d, WindowGridDirection::Right));

        assert!(!grid.has_neighbor_on_side(a, WindowGridDirection::Left));
        assert!(!grid.has_neighbor_on_side(a, WindowGridDirection::Above));
        assert!(grid.has_neighbor_on_side(a, WindowGridDirection::Right));
        assert!(grid.has_neighbor_on_side(a, WindowGridDirection::Below));

        assert!(grid.has_neighbor_on_side(b, WindowGridDirection::Left));
        assert!(!grid.has_neighbor_on_side(b, WindowGridDirection::Above));
        assert!(!grid.has_neighbor_on_side(b, WindowGridDirection::Right));

        assert!(grid.has_neighbor_on_side(c, WindowGridDirection::Above));
        assert!(grid.has_neighbor_on_side(c, WindowGridDirection::Right));
        assert!(grid.has_neighbor_on_side(d, WindowGridDirection::Left));
        assert!(!grid.has_neighbor_on_side(d, WindowGridDirection::Above));
    }

    #[test]
    fn replace_all_links_builds_typed_header_and_group_topology_transactionally() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let header_a = WindowGridMember::Header(node_ids.insert(()));
        let group_a1 = WindowGridMember::Group(node_ids.insert(()));
        let group_a2 = WindowGridMember::Group(node_ids.insert(()));
        let header_b = WindowGridMember::Header(node_ids.insert(()));
        let group_b = WindowGridMember::Group(node_ids.insert(()));
        let links = [
            WindowGridLink {
                from: header_a,
                to: group_a1,
                direction: WindowGridDirection::Below,
            },
            WindowGridLink {
                from: group_a1,
                to: group_a2,
                direction: WindowGridDirection::Below,
            },
            WindowGridLink {
                from: header_a,
                to: header_b,
                direction: WindowGridDirection::Right,
            },
            WindowGridLink {
                from: header_b,
                to: group_b,
                direction: WindowGridDirection::Below,
            },
        ];
        let mut registry = WindowGridRegistry::default();

        assert!(registry.replace_all_links(&links));
        assert_eq!(
            registry.members_for(header_a),
            vec![header_a, group_a1, group_a2, header_b, group_b],
        );
        assert_eq!(registry.all_links().len(), links.len());

        let original = registry.clone();
        let cyclic = [
            links[0],
            links[1],
            links[2],
            links[3],
            WindowGridLink {
                from: group_b,
                to: header_a,
                direction: WindowGridDirection::Right,
            },
        ];
        assert!(!registry.replace_all_links(&cyclic));
        assert_eq!(registry.all_links(), original.all_links());
        assert!(registry.is_valid());
    }

    #[test]
    fn only_linked_windows_are_marked_as_chained() {
        let [a, b, standalone] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(!grid.is_chained(a));
        assert!(grid.link_after(a, b));
        assert!(grid.is_chained(a));
        assert!(grid.is_chained(b));
        assert!(!grid.is_chained(standalone));

        assert!(grid.remove_member(a));
        assert!(!grid.is_chained(b));
    }

    #[test]
    fn direct_middle_member_removal_splits_until_geometry_is_validated() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        assert!(grid.remove_member(b));
        assert_eq!(grid.members_for(a), vec![a]);
        assert_eq!(grid.members_for(c), vec![c]);
        assert_eq!(grid.direction_between(a, c), None);
        assert_eq!(grid.members_for(b), vec![b]);
        assert!(grid.is_valid());
    }

    #[test]
    fn removing_the_first_member_rekeys_the_remaining_component() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        assert!(grid.remove_member(a));
        assert_eq!(grid.members_for(b), vec![b, c]);
        assert_eq!(grid.members_for(c), vec![b, c]);
        assert_eq!(grid.members_for(a), vec![a]);
        assert!(grid.is_valid());
    }

    #[test]
    fn removing_a_member_at_a_branch_does_not_invent_an_adjacency() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_to_side(b, c, WindowGridDirection::Below));
        assert!(grid.link_to_side(b, d, WindowGridDirection::Right));
        assert!(grid.remove_member(b));

        assert!(!grid.is_chained(a));
        assert!(!grid.is_chained(c));
        assert!(!grid.is_chained(d));
        assert!(grid.is_valid());
    }

    #[test]
    fn pruning_missing_middle_group_returns_a_straight_reconnect_candidate() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        assert!(grid.link_after(c, d));

        let live = HashSet::from([a, c, d]);
        let pruned = grid.retain_members(&live);
        assert_eq!(pruned.removed_members, 1);
        assert_eq!(
            pruned.reconnect_candidates,
            vec![super::WindowGridLink {
                from: a,
                to: c,
                direction: WindowGridDirection::Right,
            }],
        );
        assert_eq!(pruned.components_to_align, vec![c]);
        assert_eq!(grid.members_for(a), vec![a]);
        assert_eq!(grid.members_for(c), vec![c, d]);
        assert!(grid.is_valid());

        let repeated = grid.retain_members(&live);
        assert_eq!(repeated.removed_members, 0);
        assert!(repeated.reconnect_candidates.is_empty());
        assert!(repeated.components_to_align.is_empty());
    }

    #[test]
    fn pruning_a_vertical_middle_group_returns_a_below_reconnect_candidate() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_to_side(a, b, WindowGridDirection::Below));
        assert!(grid.link_to_side(b, c, WindowGridDirection::Below));

        let live = HashSet::from([a, c]);
        let pruned = grid.retain_members(&live);
        assert_eq!(
            pruned.reconnect_candidates,
            vec![super::WindowGridLink {
                from: a,
                to: c,
                direction: WindowGridDirection::Below,
            }],
        );
        assert_eq!(grid.members_for(a), vec![a]);
        assert_eq!(grid.members_for(c), vec![c]);
    }

    #[test]
    fn pruning_a_straight_run_of_missing_members_produces_one_live_reconnect() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        assert!(grid.link_after(c, d));

        let live = HashSet::from([a, d]);
        let pruned = grid.retain_members(&live);
        assert_eq!(pruned.removed_members, 2);
        assert_eq!(
            pruned.reconnect_candidates,
            vec![super::WindowGridLink {
                from: a,
                to: d,
                direction: WindowGridDirection::Right,
            }],
        );
        assert_eq!(grid.members_for(a), vec![a]);
        assert_eq!(grid.members_for(d), vec![d]);
        assert!(grid.is_valid());
    }

    #[test]
    fn pruning_missing_groups_splits_and_preserves_other_components() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_to_side(b, c, WindowGridDirection::Below));
        assert!(grid.link_to_side(c, d, WindowGridDirection::Right));

        let live = HashSet::from([a, c, d]);
        let pruned = grid.retain_members(&live);
        assert_eq!(pruned.removed_members, 1);
        assert!(pruned.reconnect_candidates.is_empty());
        assert_eq!(pruned.components_to_align, vec![c]);
        assert_eq!(grid.members_for(c), vec![c, d]);
        assert_eq!(grid.direction_between(c, d), Some(WindowGridDirection::Right));
        assert!(!grid.is_chained(a));
        assert!(grid.is_valid());
    }

    #[test]
    fn pruning_dissolves_short_components_and_is_idempotent() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));

        let live = HashSet::from([c]);
        let first = grid.retain_members(&live);
        assert_eq!(first.removed_members, 2);
        assert!(first.reconnect_candidates.is_empty());
        assert!(first.components_to_align.is_empty());

        let second = grid.retain_members(&live);
        assert_eq!(second.removed_members, 0);
        assert!(second.reconnect_candidates.is_empty());
        assert_eq!(grid.members_for(c), vec![c]);
        assert!(grid.is_valid());
    }
}
