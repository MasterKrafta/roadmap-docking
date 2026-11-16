//! Logical membership and adjacency for independent native Floating windows.
//!
//! A WindowGrid stores only relationships between windows. It does not own
//! native handles, window bounds, or Core layout state.

use std::collections::{HashMap, HashSet, VecDeque};

use dock_core::FloatingGroupId;

/// The side on which `to` is located relative to `from`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowGridDirection {
    Left,
    Right,
    Above,
    Below,
}

impl WindowGridDirection {
    fn opposite(self) -> Self {
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
    pub(crate) from: FloatingGroupId,
    pub(crate) to: FloatingGroupId,
    pub(crate) direction: WindowGridDirection,
}

impl WindowGridLink {
    fn new(
        from: FloatingGroupId,
        to: FloatingGroupId,
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
    members: Vec<FloatingGroupId>,
    links: Vec<WindowGridLink>,
}

impl WindowGrid {
    fn single(group: FloatingGroupId) -> Self {
        Self {
            members: vec![group],
            links: Vec::new(),
        }
    }

    fn direction_between(
        &self,
        from: FloatingGroupId,
        to: FloatingGroupId,
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
        preferred: FloatingGroupId,
    ) -> Option<FloatingGroupId> {
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

    /// Removes a member and reconnects a simple straight-through pair.
    ///
    /// A two-neighbour horizontal or vertical chain remains contiguous. At a
    /// branch/corner there is no unambiguous replacement edge, so the remaining
    /// components are kept separate instead of inventing a new relationship.
    fn remove_member(&mut self, group: FloatingGroupId) -> bool {
        if !self.members.contains(&group) {
            return false;
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

        if let [(first, first_side), (second, second_side)] = incident.as_slice()
            && first_side.opposite() == *second_side
        {
            let replacement = match first_side {
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
            };
            if let Some(replacement) = replacement
                && self.direction_between(replacement.from, replacement.to).is_none()
            {
                self.links.push(replacement);
            }
        }

        self.members.retain(|member| *member != group);
        self.links
            .retain(|link| link.from != group && link.to != group);
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

    fn is_ordered_horizontal_chain(&self) -> bool {
        self.members.len() > 1
            && self.links.len() == self.members.len() - 1
            && self.members.windows(2).all(|pair| {
                self.direction_between(pair[0], pair[1])
                    == Some(WindowGridDirection::Right)
            })
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

/// Registry of all linked Floating window components.
///
/// Standalone windows are intentionally implicit and are returned as
/// one-member results from `members_for`. This mirrors the previous
/// WindowChainRegistry behavior while allowing each component to hold 2D links.
#[derive(Clone, Debug, Default)]
pub(crate) struct WindowGridRegistry {
    grids: HashMap<FloatingGroupId, WindowGrid>,
    grid_by_group: HashMap<FloatingGroupId, FloatingGroupId>,
}

impl WindowGridRegistry {
    pub(crate) fn members_for(&self, group: FloatingGroupId) -> Vec<FloatingGroupId> {
        self.grid_by_group
            .get(&group)
            .and_then(|grid_id| self.grids.get(grid_id))
            .map_or_else(|| vec![group], |grid| grid.members.clone())
    }

    pub(crate) fn is_chained(&self, group: FloatingGroupId) -> bool {
        self.grid_by_group.contains_key(&group)
    }

    /// Keeps the existing left/right insertion API for current callers.
    pub(crate) fn link_before(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
    ) -> bool {
        self.link_to_side(anchor, moving, WindowGridDirection::Left)
    }

    /// Keeps the existing left/right insertion API for current callers.
    pub(crate) fn link_after(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
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
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
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
        anchor: FloatingGroupId,
        moving: &[FloatingGroupId],
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

    pub(crate) fn remove_member(&mut self, group: FloatingGroupId) -> bool {
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

    /// Prunes groups that disappeared from Core Layout.
    ///
    /// Straight horizontal chains are returned for the existing native
    /// compaction pass. Other 2D components retain their surviving links and
    /// are split only when removal disconnects the graph.
    pub(crate) fn retain_groups(
        &mut self,
        live_groups: &HashSet<FloatingGroupId>,
    ) -> (usize, Vec<Vec<FloatingGroupId>>) {
        let previous_grids = std::mem::take(&mut self.grids)
            .into_values()
            .collect::<Vec<_>>();
        self.grid_by_group.clear();

        let mut removed = 0;
        let mut chains_to_compact = Vec::new();

        for mut grid in previous_grids {
            let stale_members = grid
                .members
                .iter()
                .copied()
                .filter(|member| !live_groups.contains(member))
                .collect::<Vec<_>>();
            removed += stale_members.len();

            let had_stale_members = !stale_members.is_empty();
            for stale in &stale_members {
                grid.remove_member(*stale);
            }

            for component in grid.connected_components() {
                if had_stale_members && component.is_ordered_horizontal_chain() {
                    chains_to_compact.push(component.members.clone());
                }
                self.store_grid(component);
            }
        }

        debug_assert!(self.is_valid());
        (removed, chains_to_compact)
    }

    /// Provides a stable snapshot of a component's explicit adjacency links.
    #[allow(dead_code)] // Consumed by geometry, Z-order, and lifecycle stages.
    pub(crate) fn links_for(
        &self,
        group: FloatingGroupId,
    ) -> Option<&[WindowGridLink]> {
        let grid_id = self.grid_by_group.get(&group)?;
        self.grids.get(grid_id).map(|grid| grid.links.as_slice())
    }

    /// Returns the relative side of `to` from `from`, if they are linked.
    #[allow(dead_code)] // Used by the geometry and four-direction Drop stages.
    pub(crate) fn direction_between(
        &self,
        from: FloatingGroupId,
        to: FloatingGroupId,
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
        group: FloatingGroupId,
        side: WindowGridDirection,
    ) -> bool {
        self.links_for(group).is_some_and(|links| {
            links.iter().any(|link| {
                let relative = if link.from == group {
                    link.direction
                } else {
                    link.direction.opposite()
                };
                relative == side
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

    fn take_grid_for(&mut self, group: FloatingGroupId) -> Option<WindowGrid> {
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
    use super::{WindowGridDirection, WindowGridRegistry};
    use dock_core::FloatingGroupId;
    use slotmap::SlotMap;
    use std::collections::HashSet;

    fn groups(count: usize) -> Vec<FloatingGroupId> {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        (0..count).map(|_| ids.insert(())).collect()
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
        assert!(!grid.has_neighbor_on_side(d, WindowGridDirection::Left));
        assert!(grid.has_neighbor_on_side(d, WindowGridDirection::Left) == false);
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
    fn removing_a_middle_horizontal_member_preserves_contiguity_order() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        assert!(grid.remove_member(b));
        assert_eq!(grid.members_for(a), vec![a, c]);
        assert_eq!(grid.members_for(c), vec![a, c]);
        assert_eq!(grid.direction_between(a, c), Some(WindowGridDirection::Right));
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
    fn pruning_missing_groups_compacts_straight_horizontal_chains() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut grid = WindowGridRegistry::default();

        assert!(grid.link_after(a, b));
        assert!(grid.link_after(b, c));
        assert!(grid.link_after(c, d));

        let live = HashSet::from([a, c, d]);
        let (removed, compact) = grid.retain_groups(&live);
        assert_eq!(removed, 1);
        assert_eq!(compact, vec![vec![a, c, d]]);
        assert_eq!(grid.members_for(c), vec![a, c, d]);
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
        let (removed, compact) = grid.retain_groups(&live);
        assert_eq!(removed, 1);
        assert_eq!(compact, vec![vec![c, d]]);
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
        assert_eq!(grid.retain_groups(&live), (2, Vec::new()));
        assert_eq!(grid.retain_groups(&live), (0, Vec::new()));
        assert_eq!(grid.members_for(c), vec![c]);
        assert!(grid.is_valid());
    }
}
