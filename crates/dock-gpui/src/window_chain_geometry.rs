//! Pure geometry and ordering helpers for native Floating window grids.
//!
//! This module intentionally has no GPUI Entity or NativeWindowOps access.
//! The initial extraction preserves the existing horizontal calculations;
//! vertical placement and grid-wide resize geometry are added in later stages.

use std::collections::{HashMap, HashSet, VecDeque};

use dock_core::{FloatingGroupId, Point, Rect, Size};
use platform::{PhysicalPoint, PhysicalRect, PhysicalSize};

use crate::window_chain::{WindowGridDirection, WindowGridLink, WindowGridRegistry};

/// Native frame and client measurements, all in physical desktop pixels.
///
/// Core stores each window's outer origin in physical pixels and client size in
/// logical pixels. Keeping the native client insets and scale beside the Core
/// bounds lets vertical header overlap be calculated without mixing DPI spaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeFrameGeometry {
    pub(crate) outer_width: f32,
    pub(crate) outer_height: f32,
    pub(crate) visible_left_inset: f32,
    pub(crate) visible_right_inset: f32,
    pub(crate) client_left_inset: f32,
    pub(crate) client_top_inset: f32,
    pub(crate) client_right_inset: f32,
    pub(crate) client_bottom_inset: f32,
    pub(crate) client_width: f32,
    pub(crate) client_height: f32,
    pub(crate) scale_factor: f32,
}

impl NativeFrameGeometry {
    pub(crate) fn from_native_rects(
        outer: PhysicalRect,
        visible: PhysicalRect,
        client_origin: PhysicalPoint,
        client_size: PhysicalSize,
        scale_factor: f32,
    ) -> Option<Self> {
        let outer_width = i64::from(outer.right) - i64::from(outer.left);
        let outer_height = i64::from(outer.bottom) - i64::from(outer.top);
        let visible_left_inset = i64::from(visible.left) - i64::from(outer.left);
        let visible_right_inset = i64::from(outer.right) - i64::from(visible.right);
        let client_left_inset = i64::from(client_origin.x) - i64::from(outer.left);
        let client_top_inset = i64::from(client_origin.y) - i64::from(outer.top);
        let client_right_inset = i64::from(outer.right)
            - (i64::from(client_origin.x) + i64::from(client_size.width));
        let client_bottom_inset = i64::from(outer.bottom)
            - (i64::from(client_origin.y) + i64::from(client_size.height));

        if outer_width <= 0
            || outer_height <= 0
            || !client_size.is_valid()
            || !scale_factor.is_finite()
            || scale_factor <= 0.0
            || visible_left_inset < 0
            || visible_right_inset < 0
            || visible_left_inset + visible_right_inset >= outer_width
            || client_left_inset < 0
            || client_top_inset < 0
            || client_right_inset < 0
            || client_bottom_inset < 0
            || client_left_inset + i64::from(client_size.width) + client_right_inset
                != outer_width
            || client_top_inset + i64::from(client_size.height) + client_bottom_inset
                != outer_height
        {
            return None;
        }

        Some(Self {
            outer_width: outer_width as f32,
            outer_height: outer_height as f32,
            visible_left_inset: visible_left_inset as f32,
            visible_right_inset: visible_right_inset as f32,
            client_left_inset: client_left_inset as f32,
            client_top_inset: client_top_inset as f32,
            client_right_inset: client_right_inset as f32,
            client_bottom_inset: client_bottom_inset as f32,
            client_width: client_size.width as f32,
            client_height: client_size.height as f32,
            scale_factor,
        })
    }

    /// Fallback for a not-yet-created HWND. Its future client is treated as a
    /// frame-less window; reconciliation replaces this estimate with native data.
    pub(crate) fn without_frame_size(
        logical_size: Size,
        scale_factor: f32,
    ) -> Option<Self> {
        if !logical_size.width.is_finite()
            || logical_size.width <= 0.0
            || !logical_size.height.is_finite()
            || logical_size.height <= 0.0
            || !scale_factor.is_finite()
            || scale_factor <= 0.0
        {
            return None;
        }

        let outer_width = logical_size.width * scale_factor;
        let outer_height = logical_size.height * scale_factor;
        let geometry = Self {
            outer_width,
            outer_height,
            visible_left_inset: 0.0,
            visible_right_inset: 0.0,
            client_left_inset: 0.0,
            client_top_inset: 0.0,
            client_right_inset: 0.0,
            client_bottom_inset: 0.0,
            client_width: outer_width,
            client_height: outer_height,
            scale_factor,
        };
        geometry.is_valid().then_some(geometry)
    }

    /// Horizontal-only compatibility helper for tests without a real HWND.
    pub(crate) fn without_frame(outer_width: f32) -> Option<Self> {
        Self::without_frame_size(Size::new(outer_width, 1.0), 1.0)
    }

    fn is_valid(self) -> bool {
        self.outer_width.is_finite()
            && self.outer_width > 0.0
            && self.outer_height.is_finite()
            && self.outer_height > 0.0
            && self.visible_left_inset.is_finite()
            && self.visible_right_inset.is_finite()
            && self.visible_left_inset >= 0.0
            && self.visible_right_inset >= 0.0
            && self.visible_left_inset + self.visible_right_inset < self.outer_width
            && self.client_left_inset.is_finite()
            && self.client_top_inset.is_finite()
            && self.client_right_inset.is_finite()
            && self.client_bottom_inset.is_finite()
            && self.client_left_inset >= 0.0
            && self.client_top_inset >= 0.0
            && self.client_right_inset >= 0.0
            && self.client_bottom_inset >= 0.0
            && self.client_width.is_finite()
            && self.client_width > 0.0
            && self.client_height.is_finite()
            && self.client_height > 0.0
            && self.scale_factor.is_finite()
            && self.scale_factor > 0.0
            && (self.client_left_inset + self.client_width + self.client_right_inset
                - self.outer_width)
                .abs()
                < 0.5
            && (self.client_top_inset + self.client_height + self.client_bottom_inset
                - self.outer_height)
                .abs()
                < 0.5
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

/// Calculates every member's physical desktop origin from the WindowGrid links.
///
/// The anchor remains fixed. Horizontal neighbors meet at their DWM-visible
/// frame edges. Vertical neighbors align their client left edges and overlap
/// exactly the lower window's CompositionHeader height, so the upper HWND
/// covers the lower header. Repeated links on one side are laid out as a
/// deterministic outward lane in member order.
pub(crate) fn arrange_window_grid_bounds(
    mut bounds: Vec<(FloatingGroupId, Rect)>,
    frames: &[NativeFrameGeometry],
    links: &[WindowGridLink],
    anchor: FloatingGroupId,
    header_height: f32,
) -> Option<Vec<(FloatingGroupId, Rect)>> {
    if bounds.len() != frames.len()
        || bounds.is_empty()
        || frames.iter().any(|frame| !frame.is_valid())
        || !header_height.is_finite()
        || header_height <= 0.0
    {
        return None;
    }

    let mut indices = HashMap::with_capacity(bounds.len());
    for (index, (group, rect)) in bounds.iter().enumerate() {
        if indices.insert(*group, index).is_some()
            || !rect.origin.x.is_finite()
            || !rect.origin.y.is_finite()
            || !rect.size.width.is_finite()
            || rect.size.width <= 0.0
            || !rect.size.height.is_finite()
            || rect.size.height <= 0.0
        {
            return None;
        }
    }

    let anchor_index = *indices.get(&anchor)?;
    if bounds.len() == 1 {
        return links.is_empty().then_some(bounds);
    }
    if links.len() != bounds.len() - 1 {
        return None;
    }

    let mut adjacency = vec![Vec::<(usize, WindowGridDirection)>::new(); bounds.len()];
    let mut edges = HashSet::new();
    for link in links {
        let from = *indices.get(&link.from)?;
        let to = *indices.get(&link.to)?;
        if from == to
            || !edges.insert(if from < to { (from, to) } else { (to, from) })
        {
            return None;
        }
        adjacency[from].push((to, link.direction));
        adjacency[to].push((from, opposite_direction(link.direction)));
    }

    let mut visited = HashSet::from([anchor_index]);
    let mut pending = VecDeque::from([anchor_index]);
    // These are the physical neighbors after same-side branches are packed
    // into one outward lane. They are used to reject unrelated window overlap.
    let mut physical_edges = HashSet::new();

    while let Some(current) = pending.pop_front() {
        for direction in [
            WindowGridDirection::Left,
            WindowGridDirection::Right,
            WindowGridDirection::Above,
            WindowGridDirection::Below,
        ] {
            let mut neighbours = adjacency[current]
                .iter()
                .filter_map(|(index, candidate_direction)| {
                    (*candidate_direction == direction && !visited.contains(index))
                        .then_some(*index)
                })
                .collect::<Vec<_>>();

            // The nearest member stays beside the target. Additional members
            // on that same side extend the lane outward from the previous one.
            neighbours.sort_by_key(|index| match direction {
                WindowGridDirection::Left | WindowGridDirection::Above => {
                    std::cmp::Reverse(*index)
                }
                WindowGridDirection::Right | WindowGridDirection::Below => *index,
            });

            let mut previous = current;
            for neighbour in neighbours {
                place_grid_neighbour(
                    &bounds[previous].1,
                    &frames[previous],
                    &mut bounds[neighbour].1,
                    &frames[neighbour],
                    direction,
                    header_height,
                )?;
                if !visited.insert(neighbour) {
                    return None;
                }
                physical_edges.insert(if previous < neighbour {
                    (previous, neighbour)
                } else {
                    (neighbour, previous)
                });
                pending.push_back(neighbour);
                previous = neighbour;
            }
        }
    }

    if visited.len() != bounds.len() {
        return None;
    }

    // An unrelated pair must not overlap in client space. Intentional vertical
    // header overlap is allowed only for the physical adjacent pairs above.
    for first in 0..bounds.len() {
        for second in first + 1..bounds.len() {
            if physical_edges.contains(&(first, second)) {
                continue;
            }
            if client_rects_overlap(
                bounds[first].1,
                frames[first],
                bounds[second].1,
                frames[second],
            ) {
                return None;
            }
        }
    }

    Some(bounds)
}

fn opposite_direction(direction: WindowGridDirection) -> WindowGridDirection {
    match direction {
        WindowGridDirection::Left => WindowGridDirection::Right,
        WindowGridDirection::Right => WindowGridDirection::Left,
        WindowGridDirection::Above => WindowGridDirection::Below,
        WindowGridDirection::Below => WindowGridDirection::Above,
    }
}

fn place_grid_neighbour(
    from_bounds: &Rect,
    from_frame: &NativeFrameGeometry,
    to_bounds: &mut Rect,
    to_frame: &NativeFrameGeometry,
    direction: WindowGridDirection,
    header_height: f32,
) -> Option<()> {
    let x = match direction {
        WindowGridDirection::Left => {
            from_bounds.origin.x + from_frame.visible_left_inset
                - to_frame.outer_width
                + to_frame.visible_right_inset
        }
        WindowGridDirection::Right => {
            from_bounds.origin.x + from_frame.outer_width
                - from_frame.visible_right_inset
                - to_frame.visible_left_inset
        }
        WindowGridDirection::Above | WindowGridDirection::Below => {
            from_bounds.origin.x + from_frame.client_left_inset - to_frame.client_left_inset
        }
    };

    let y = match direction {
        WindowGridDirection::Left | WindowGridDirection::Right => from_bounds.origin.y,
        WindowGridDirection::Below => {
            let from_client_bottom = from_bounds.origin.y
                + from_frame.client_top_inset
                + from_frame.client_height;
            let overlap = header_height * to_frame.scale_factor;
            from_client_bottom - overlap - to_frame.client_top_inset
        }
        WindowGridDirection::Above => {
            let from_client_top = from_bounds.origin.y + from_frame.client_top_inset;
            let overlap = header_height * from_frame.scale_factor;
            from_client_top + overlap
                - to_frame.client_top_inset
                - to_frame.client_height
        }
    };

    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    to_bounds.origin = Point::new(x, y);
    Some(())
}

fn client_rects_overlap(
    first_bounds: Rect,
    first_frame: NativeFrameGeometry,
    second_bounds: Rect,
    second_frame: NativeFrameGeometry,
) -> bool {
    let first_left = first_bounds.origin.x + first_frame.client_left_inset;
    let first_top = first_bounds.origin.y + first_frame.client_top_inset;
    let first_right = first_left + first_frame.client_width;
    let first_bottom = first_top + first_frame.client_height;

    let second_left = second_bounds.origin.x + second_frame.client_left_inset;
    let second_top = second_bounds.origin.y + second_frame.client_top_inset;
    let second_right = second_left + second_frame.client_width;
    let second_bottom = second_top + second_frame.client_height;

    first_left < second_right
        && first_right > second_left
        && first_top < second_bottom
        && first_bottom > second_top
}

/// Normalizes only the moving window's logical width to the target window.
///
/// This is used for a vertical Drop: the target window remains the width
/// reference and is never resized to accommodate the moving window.
pub(crate) fn match_moving_window_width_to_target(
    mut bounds: Vec<(FloatingGroupId, Rect)>,
    target: FloatingGroupId,
    moving: FloatingGroupId,
) -> Option<Vec<(FloatingGroupId, Rect)>> {
    let target_index = bounds.iter().position(|(group, _)| *group == target)?;
    let moving_index = bounds.iter().position(|(group, _)| *group == moving)?;
    let target_width = bounds[target_index].1.size.width;
    if !target_width.is_finite() || target_width <= 0.0 {
        return None;
    }
    bounds[moving_index].1.size.width = target_width;
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
    use slotmap::{Key, SlotMap};

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

#[cfg(test)]
mod moving_window_width_tests {
    use super::match_moving_window_width_to_target;
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [FloatingGroupId; 2]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [ids.insert(()), ids.insert(())];
        (ids, groups)
    }

    #[test]
    fn moving_window_wider_than_target_is_narrowed_without_resizing_target() {
        let (_ids, [target, moving]) = groups();
        let target_bounds = Rect::new(Point::new(20.0, 30.0), Size::new(240.0, 300.0));
        let moving_bounds = Rect::new(Point::new(800.0, 50.0), Size::new(420.0, 500.0));

        let result = match_moving_window_width_to_target(
            vec![(target, target_bounds), (moving, moving_bounds)],
            target,
            moving,
        )
        .expect("both windows must exist");

        assert_eq!(result[0].1, target_bounds);
        assert_eq!(result[1].1.size.width, 240.0);
        assert_eq!(result[1].1.size.height, moving_bounds.size.height);
        assert_eq!(result[1].1.origin, moving_bounds.origin);
    }

    #[test]
    fn moving_window_narrower_than_target_is_widened_to_target_width() {
        let (_ids, [target, moving]) = groups();
        let target_bounds = Rect::new(Point::new(20.0, 30.0), Size::new(360.0, 300.0));
        let moving_bounds = Rect::new(Point::new(800.0, 50.0), Size::new(180.0, 500.0));

        let result = match_moving_window_width_to_target(
            vec![(target, target_bounds), (moving, moving_bounds)],
            target,
            moving,
        )
        .expect("both windows must exist");

        assert_eq!(result[0].1, target_bounds);
        assert_eq!(result[1].1.size.width, 360.0);
        assert_eq!(result[1].1.size.height, moving_bounds.size.height);
    }
}

