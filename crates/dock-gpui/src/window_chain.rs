//! Logical membership and adjacency for independent native Floating windows.
//!
//! A WindowGrid stores only relationships between windows. It does not own
//! native handles, window bounds, or Core layout state.

#[path = "window_grid_member.rs"]
mod window_grid_member;

pub(crate) use window_grid_member::WindowGridMember;

#[path = "window_grid_direction.rs"]
mod window_grid_direction;

pub(crate) use window_grid_direction::WindowGridDirection;

#[path = "window_grid_link.rs"]
mod window_grid_link;

pub(crate) use window_grid_link::WindowGridLink;

#[path = "window_grid.rs"]
mod window_grid;

use window_grid::WindowGrid;


#[path = "window_grid_prune_result.rs"]
mod window_grid_prune_result;

pub(crate) use window_grid_prune_result::WindowGridPruneResult;

#[path = "window_grid_registry.rs"]
mod window_grid_registry;

pub(crate) use window_grid_registry::WindowGridRegistry;


#[cfg(test)]
mod tests {
    use super::{WindowGridDirection, WindowGridLink, WindowGridMember, WindowGridRegistry};
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
        let members = registry.members_for(header_a);
        assert_eq!(members.len(), 5);
        assert!(members.contains(&header_a));
        assert!(members.contains(&group_a1));
        assert!(members.contains(&group_a2));
        assert!(members.contains(&header_b));
        assert!(members.contains(&group_b));
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
