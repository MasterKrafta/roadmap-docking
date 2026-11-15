//! Pure geometry and ordering helpers for native Floating window grids.
//!
//! This module intentionally has no GPUI Entity or NativeWindowOps access.
//! The initial extraction preserves the existing horizontal calculations;
//! vertical placement and grid-wide resize geometry are added in later stages.

use std::collections::HashSet;

use dock_core::{FloatingGroupId, Rect};
use platform::PhysicalRect;

use crate::window_chain::WindowGridRegistry;

/// Native outer width and DWM-visible insets, all expressed in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeFrameGeometry {
    pub(crate) outer_width: f32,
    pub(crate) visible_left_inset: f32,
    pub(crate) visible_right_inset: f32,
}

impl NativeFrameGeometry {
    pub(crate) fn from_rects(outer: PhysicalRect, visible: PhysicalRect) -> Option<Self> {
        let outer_width = i64::from(outer.right) - i64::from(outer.left);
        let visible_left_inset = i64::from(visible.left) - i64::from(outer.left);
        let visible_right_inset = i64::from(outer.right) - i64::from(visible.right);

        if outer_width <= 0
            || visible_left_inset < 0
            || visible_right_inset < 0
            || visible_left_inset + visible_right_inset >= outer_width
        {
            return None;
        }

        Some(Self {
            outer_width: outer_width as f32,
            visible_left_inset: visible_left_inset as f32,
            visible_right_inset: visible_right_inset as f32,
        })
    }

    pub(crate) fn without_frame(outer_width: f32) -> Option<Self> {
        (outer_width.is_finite() && outer_width > 0.0).then_some(Self {
            outer_width,
            visible_left_inset: 0.0,
            visible_right_inset: 0.0,
        })
    }

    fn is_valid(self) -> bool {
        self.outer_width.is_finite()
            && self.outer_width > 0.0
            && self.visible_left_inset.is_finite()
            && self.visible_right_inset.is_finite()
            && self.visible_left_inset >= 0.0
            && self.visible_right_inset >= 0.0
            && self.visible_left_inset + self.visible_right_inset < self.outer_width
    }
}

/// Keeps the anchor at its drop position and aligns a horizontal chain by its
/// DWM-visible edges. Each member retains its own Core size.
pub(crate) fn arrange_contiguous_window_chain_bounds(
    mut bounds: Vec<(FloatingGroupId, Rect)>,
    frames: &[NativeFrameGeometry],
    anchor: FloatingGroupId,
) -> Option<Vec<(FloatingGroupId, Rect)>> {
    if bounds.len() != frames.len() || frames.iter().any(|frame| !frame.is_valid()) {
        return None;
    }

    let anchor_index = bounds.iter().position(|(group, _)| *group == anchor)?;
    let anchor_origin = bounds[anchor_index].1.origin;

    for index in (0..anchor_index).rev() {
        let next = frames[index + 1];
        let current = frames[index];
        bounds[index].1.origin.x = bounds[index + 1].1.origin.x
            + next.visible_left_inset
            - current.outer_width
            + current.visible_right_inset;
        bounds[index].1.origin.y = anchor_origin.y;
    }
    for index in anchor_index + 1..bounds.len() {
        let previous = frames[index - 1];
        let current = frames[index];
        bounds[index].1.origin.x = bounds[index - 1].1.origin.x
            + previous.outer_width
            - previous.visible_right_inset
            - current.visible_left_inset;
        bounds[index].1.origin.y = anchor_origin.y;
    }

    Some(bounds)
}

/// Returns the change in a resized window's right edge in physical pixels.
pub(crate) fn native_resize_right_edge_delta(
    previous: Rect,
    actual: Rect,
    scale_factor: f32,
) -> Option<f32> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let previous_right = previous.origin.x + previous.size.width * scale_factor;
    let actual_right = actual.origin.x + actual.size.width * scale_factor;
    let delta = actual_right - previous_right;
    delta.is_finite().then_some(delta)
}

/// Shifts only members after the resized window. Origins are physical desktop
/// pixels; every member's own logical dimensions stay intact.
pub(crate) fn shift_window_chain_after_resize(
    mut bounds: Vec<(FloatingGroupId, Rect)>,
    resized_group: FloatingGroupId,
    right_edge_delta: f32,
) -> Option<Vec<(FloatingGroupId, Rect)>> {
    if !right_edge_delta.is_finite() {
        return None;
    }
    let resized_index = bounds
        .iter()
        .position(|(group, _)| *group == resized_group)?;
    for (_, member_bounds) in bounds.iter_mut().skip(resized_index + 1) {
        member_bounds.origin.x += right_edge_delta;
        if !member_bounds.origin.x.is_finite() {
            return None;
        }
    }
    Some(bounds)
}

/// Produces a deterministic back-to-front order while keeping each connected
/// component adjacent in the native Z stack.
pub(crate) fn floating_groups_in_native_z_order(
    groups: &[(FloatingGroupId, i32)],
    grids: &WindowGridRegistry,
) -> Vec<FloatingGroupId> {
    let mut emitted = HashSet::new();
    let mut blocks = Vec::new();

    for (group_id, group_z_order) in groups {
        if emitted.contains(group_id) {
            continue;
        }

        let block = grids
            .members_for(*group_id)
            .into_iter()
            .filter(|member| {
                !emitted.contains(member) && groups.iter().any(|(candidate, _)| candidate == member)
            })
            .collect::<Vec<_>>();
        if block.is_empty() {
            continue;
        }

        let block_z_order = block
            .iter()
            .filter_map(|member| {
                groups
                    .iter()
                    .find(|(candidate, _)| candidate == member)
                    .map(|(_, z_order)| *z_order)
            })
            .max()
            .unwrap_or(*group_z_order);

        emitted.extend(block.iter().copied());
        blocks.push((block_z_order, block));
    }

    blocks.sort_by_key(|(z_order, _)| *z_order);
    blocks
        .into_iter()
        .flat_map(|(_, mut members)| {
            // HWND_TOP is applied back-to-front: the left/top member is raised last.
            members.reverse();
            members
        })
        .collect()
}

#[cfg(test)]
mod contiguous_window_chain_bounds_tests {
    use super::{NativeFrameGeometry, arrange_contiguous_window_chain_bounds};
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [FloatingGroupId; 3]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [ids.insert(()), ids.insert(()), ids.insert(())];
        (ids, groups)
    }

    #[test]
    fn keeps_anchor_and_places_neighbours_edge_to_edge() {
        let (_ids, [a, b, c]) = groups();
        let input = vec![
            (a, Rect::new(Point::new(50.0, 10.0), Size::new(120.0, 300.0))),
            (b, Rect::new(Point::new(500.0, 40.0), Size::new(240.0, 360.0))),
            (c, Rect::new(Point::new(900.0, 90.0), Size::new(180.0, 280.0))),
        ];

        let frames = [
            NativeFrameGeometry::without_frame(120.0).unwrap(),
            NativeFrameGeometry::without_frame(240.0).unwrap(),
            NativeFrameGeometry::without_frame(180.0).unwrap(),
        ];
        let arranged = arrange_contiguous_window_chain_bounds(input, &frames, b)
            .expect("anchor must belong to the chain");

        assert_eq!(arranged[0].1.origin, Point::new(380.0, 40.0));
        assert_eq!(arranged[1].1.origin, Point::new(500.0, 40.0));
        assert_eq!(arranged[2].1.origin, Point::new(740.0, 40.0));
        assert_eq!(arranged[0].1.size, Size::new(120.0, 300.0));
        assert_eq!(arranged[1].1.size, Size::new(240.0, 360.0));
        assert_eq!(arranged[2].1.size, Size::new(180.0, 280.0));
    }

    #[test]
    fn supports_anchor_at_chain_edge_and_rejects_missing_anchor() {
        let (_ids, [a, b, c]) = groups();
        let input = vec![
            (a, Rect::new(Point::new(30.0, 8.0), Size::new(100.0, 200.0))),
            (b, Rect::new(Point::new(300.0, 28.0), Size::new(150.0, 240.0))),
            (c, Rect::new(Point::new(700.0, 48.0), Size::new(200.0, 260.0))),
        ];

        let frames = [
            NativeFrameGeometry::without_frame(100.0).unwrap(),
            NativeFrameGeometry::without_frame(150.0).unwrap(),
            NativeFrameGeometry::without_frame(200.0).unwrap(),
        ];
        let arranged = arrange_contiguous_window_chain_bounds(input.clone(), &frames, a)
            .expect("first member is a valid anchor");
        assert_eq!(arranged[0].1.origin, Point::new(30.0, 8.0));
        assert_eq!(arranged[1].1.origin, Point::new(130.0, 8.0));
        assert_eq!(arranged[2].1.origin, Point::new(280.0, 8.0));

        assert!(
            arrange_contiguous_window_chain_bounds(input, &frames, FloatingGroupId::null())
                .is_none()
        );
    }

    #[test]
    fn visible_frame_edges_touch_despite_invisible_resize_borders() {
        let (_ids, [a, b, c]) = groups();
        let input = vec![
            (a, Rect::new(Point::new(0.0, 20.0), Size::new(300.0, 240.0))),
            (b, Rect::new(Point::new(1000.0, 40.0), Size::new(400.0, 320.0))),
            (c, Rect::new(Point::new(1800.0, 60.0), Size::new(200.0, 200.0))),
        ];

        let frames = [
            NativeFrameGeometry {
                outer_width: 450.0,
                visible_left_inset: 6.0,
                visible_right_inset: 6.0,
            },
            NativeFrameGeometry {
                outer_width: 500.0,
                visible_left_inset: 8.0,
                visible_right_inset: 8.0,
            },
            NativeFrameGeometry {
                outer_width: 400.0,
                visible_left_inset: 4.0,
                visible_right_inset: 4.0,
            },
        ];
        let arranged = arrange_contiguous_window_chain_bounds(input, &frames, b)
            .expect("visible frame geometry must place the chain");
        assert_eq!(arranged[0].1.origin, Point::new(564.0, 40.0));
        assert_eq!(arranged[1].1.origin, Point::new(1000.0, 40.0));
        assert_eq!(arranged[2].1.origin, Point::new(1488.0, 40.0));

        let a_visible_right =
            arranged[0].1.origin.x + frames[0].outer_width - frames[0].visible_right_inset;
        let b_visible_left = arranged[1].1.origin.x + frames[1].visible_left_inset;
        let b_visible_right =
            arranged[1].1.origin.x + frames[1].outer_width - frames[1].visible_right_inset;
        let c_visible_left = arranged[2].1.origin.x + frames[2].visible_left_inset;
        assert_eq!(a_visible_right, b_visible_left);
        assert_eq!(b_visible_right, c_visible_left);
    }

    #[test]
    fn rejects_missing_or_invalid_frame_geometry() {
        let (_ids, [a, b, _c]) = groups();
        let input = vec![
            (a, Rect::new(Point::ZERO, Size::new(100.0, 200.0))),
            (b, Rect::new(Point::new(300.0, 0.0), Size::new(100.0, 200.0))),
        ];
        let valid = NativeFrameGeometry::without_frame(100.0).unwrap();
        assert!(arrange_contiguous_window_chain_bounds(input.clone(), &[valid], a).is_none());
        assert!(
            arrange_contiguous_window_chain_bounds(
                input,
                &[
                    valid,
                    NativeFrameGeometry {
                        outer_width: 100.0,
                        visible_left_inset: 50.0,
                        visible_right_inset: 50.0,
                    },
                ],
                a,
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod native_resize_chain_geometry_tests {
    use super::{native_resize_right_edge_delta, shift_window_chain_after_resize};
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [FloatingGroupId; 3]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [ids.insert(()), ids.insert(()), ids.insert(())];
        (ids, groups)
    }

    #[test]
    fn right_edge_resize_moves_later_windows_by_physical_width_delta_only() {
        let (_ids, [a, b, c]) = groups();
        let before = Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0));
        let actual = Rect::new(Point::new(100.0, 20.0), Size::new(340.0, 400.0));
        let delta = native_resize_right_edge_delta(before, actual, 1.5)
            .expect("valid monitor scale");
        assert_eq!(delta, 60.0);

        let bounds = vec![
            (a, Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0))),
            (b, actual),
            (c, Rect::new(Point::new(760.0, 20.0), Size::new(220.0, 320.0))),
        ];
        let shifted = shift_window_chain_after_resize(bounds, b, delta)
            .expect("resized member must belong to chain");
        assert_eq!(shifted[0].1.origin, Point::new(100.0, 20.0));
        assert_eq!(shifted[1].1.origin, Point::new(100.0, 20.0));
        assert_eq!(shifted[2].1.origin, Point::new(820.0, 20.0));
        assert_eq!(shifted[2].1.size, Size::new(220.0, 320.0));
    }

    #[test]
    fn left_edge_resize_with_anchored_right_edge_does_not_move_neighbours() {
        let before = Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0));
        let actual = Rect::new(Point::new(160.0, 20.0), Size::new(260.0, 400.0));
        assert_eq!(native_resize_right_edge_delta(before, actual, 1.5), Some(0.0));
    }

    #[test]
    fn resizing_last_member_does_not_change_earlier_windows() {
        let (_ids, [a, b, c]) = groups();
        let bounds = vec![
            (a, Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0))),
            (b, Rect::new(Point::new(400.0, 20.0), Size::new(240.0, 360.0))),
            (c, Rect::new(Point::new(640.0, 20.0), Size::new(220.0, 320.0))),
        ];
        let shifted = shift_window_chain_after_resize(bounds.clone(), c, 80.0)
            .expect("last member must belong to chain");
        assert_eq!(shifted, bounds);
    }
}

#[cfg(test)]
mod native_z_order_tests {
    use super::floating_groups_in_native_z_order;
    use crate::window_chain::WindowGridRegistry;
    use dock_core::FloatingGroupId;
    use slotmap::SlotMap;

    #[test]
    fn grid_members_are_kept_adjacent_in_native_z_order() {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let [a, b, c, d, e] = [
            ids.insert(()),
            ids.insert(()),
            ids.insert(()),
            ids.insert(()),
            ids.insert(()),
        ];
        let groups = [(a, 1), (c, 4), (d, 5), (e, 7), (b, 8)];
        let mut grids = WindowGridRegistry::default();
        assert!(grids.link_after(a, b));
        assert!(grids.link_after(c, d));

        assert_eq!(
            floating_groups_in_native_z_order(&groups, &grids),
            vec![d, c, e, b, a],
            "grid members are raised back-to-front internally and ranked by their frontmost member"
        );
    }

    #[test]
    fn an_unlinked_floating_group_remains_a_single_z_order_block() {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let [a, b, c, d] = [ids.insert(()), ids.insert(()), ids.insert(()), ids.insert(())];
        let groups = [(a, 3), (b, 1), (c, 2), (d, 4)];

        assert_eq!(
            floating_groups_in_native_z_order(&groups, &WindowGridRegistry::default()),
            vec![b, c, a, d]
        );
    }
}
