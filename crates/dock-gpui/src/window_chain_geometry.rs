//! Pure geometry and ordering helpers for native Floating window grids.
//!
//! This module intentionally has no GPUI Entity or NativeWindowOps access.
//! Native measurements are passed in as physical frame/client geometry, while
//! DockCore bounds retain their mixed physical-origin/logical-size convention.

use std::collections::{HashMap, HashSet, VecDeque};

use dock_core::{Point, Rect, Size};
use platform::{PhysicalPoint, PhysicalRect, PhysicalSize};

use crate::window_chain::{WindowGridDirection, WindowGridLink, WindowGridMember, WindowGridRegistry};

/// Native frame and client measurements, all in physical desktop pixels.
///
/// Core stores each window's outer origin in physical pixels and client size in
/// logical pixels. Keeping the native client insets and scale beside the Core
/// bounds lets every independent window align its client edges without mixing DPI spaces.
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
            || i64::from(visible.top) < i64::from(outer.top)
            || i64::from(visible.bottom) > i64::from(outer.bottom)
            || visible.bottom <= visible.top
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

    /// Recalculates the expected native outer size for a requested Core client
    /// size while preserving the measured non-client insets and this HWND's DPI.
    pub(crate) fn with_logical_client_size(mut self, logical_size: Size) -> Option<Self> {
        if !logical_size.width.is_finite()
            || logical_size.width <= 0.0
            || !logical_size.height.is_finite()
            || logical_size.height <= 0.0
        {
            return None;
        }

        self.client_width = logical_size.width * self.scale_factor;
        self.client_height = logical_size.height * self.scale_factor;
        self.outer_width =
            self.client_left_inset + self.client_width + self.client_right_inset;
        self.outer_height =
            self.client_top_inset + self.client_height + self.client_bottom_inset;
        self.is_valid().then_some(self)
    }

    /// Converts a desired physical client origin and logical client size into
    /// the outer-window bounds required by this HWND's measured frame and DPI.
    pub(crate) fn outer_bounds_for_client_rect(self, client_rect: Rect) -> Option<Rect> {
        client_rect.validate_non_empty().ok()?;
        let frame = self.with_logical_client_size(client_rect.size)?;
        let outer = Rect::new(
            Point::new(
                client_rect.origin.x - frame.client_left_inset,
                client_rect.origin.y - frame.client_top_inset,
            ),
            client_rect.size,
        );
        outer.validate_non_empty().ok()?;
        Some(outer)
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
    #[cfg(test)]
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

/// Converts one Core Floating member's local client rectangle into native
/// outer bounds. FloatingGroup stores the original composition's outer origin;
/// adding this HWND's client insets recovers the composition client origin before
/// the local logical offset is scaled. The final conversion subtracts the new
/// HWND's own insets, keeping its client area aligned with the Core rectangle.
pub(crate) fn floating_column_outer_bounds(
    floating_outer_origin: Point,
    local_origin: Point,
    logical_size: Size,
    frame: NativeFrameGeometry,
) -> Option<Rect> {
    floating_outer_origin.validate().ok()?;
    local_origin.validate().ok()?;
    let client_origin = Point::new(
        floating_outer_origin.x
            + frame.client_left_inset
            + local_origin.x * frame.scale_factor,
        floating_outer_origin.y
            + frame.client_top_inset
            + local_origin.y * frame.scale_factor,
    );
    client_origin.validate().ok()?;
    frame.outer_bounds_for_client_rect(Rect::new(client_origin, logical_size))
}

/// Keeps the anchor at its drop position and aligns a horizontal chain by its
/// DWM-visible edges. Each member retains its own Core size.
#[cfg(test)]
pub(crate) fn arrange_contiguous_window_chain_bounds(
    mut bounds: Vec<(WindowGridMember, Rect)>,
    frames: &[NativeFrameGeometry],
    anchor: WindowGridMember,
) -> Option<Vec<(WindowGridMember, Rect)>> {
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
/// frame edges. Vertical neighbors align their client left edges and touch at
/// their client top/bottom edges without overlap. Repeated links on one side
/// are laid out as a deterministic outward lane in member order.
#[cfg(test)]
fn arrange_window_grid_bounds(
    bounds: Vec<(WindowGridMember, Rect)>,
    frames: &[NativeFrameGeometry],
    links: &[WindowGridLink],
    anchor: WindowGridMember,
) -> Option<Vec<(WindowGridMember, Rect)>> {
    arrange_window_grid_bounds_with_preserved_offsets(
        bounds,
        frames,
        links,
        anchor,
        &HashSet::new(),
    )
}

/// Arranges a WindowGrid while preserving the perpendicular offset across
/// selected internal links. Floating Columns in one Core composition can have
/// independent vertical viewports, so their horizontal WindowGrid links must
/// retain their Y offset instead of forcing every Column to the same top edge.
pub(crate) fn arrange_window_grid_bounds_with_preserved_offsets(
    mut bounds: Vec<(WindowGridMember, Rect)>,
    frames: &[NativeFrameGeometry],
    links: &[WindowGridLink],
    anchor: WindowGridMember,
    preserve_parallel_offsets: &HashSet<WindowGridLink>,
) -> Option<Vec<(WindowGridMember, Rect)>> {
    if bounds.len() != frames.len()
        || bounds.is_empty()
        || frames.iter().any(|frame| !frame.is_valid())
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
    let original_bounds = bounds.clone();
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
        adjacency[to].push((from, link.direction.opposite()));
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
            neighbours.sort_by(|left, right| match direction {
                WindowGridDirection::Left | WindowGridDirection::Above => right.cmp(left),
                WindowGridDirection::Right | WindowGridDirection::Below => left.cmp(right),
            });

            let mut previous = current;
            for neighbour in neighbours {
                let preserve_parallel_offset = links.iter().any(|link| {
                    preserve_parallel_offsets.contains(link)
                        && ((link.from == bounds[previous].0 && link.to == bounds[neighbour].0)
                            || (link.to == bounds[previous].0
                                && link.from == bounds[neighbour].0))
                });
                let parallel_offset = if preserve_parallel_offset {
                    Some(match direction {
                        WindowGridDirection::Left | WindowGridDirection::Right => {
                            original_bounds[neighbour].1.origin.y
                                - original_bounds[previous].1.origin.y
                        }
                        WindowGridDirection::Above | WindowGridDirection::Below => {
                            original_bounds[neighbour].1.origin.x
                                - original_bounds[previous].1.origin.x
                        }
                    })
                } else {
                    None
                };
                let placed = if previous < neighbour {
                    let (before_neighbour, from_neighbour) = bounds.split_at_mut(neighbour);
                    place_grid_neighbour(
                        &before_neighbour[previous].1,
                        &frames[previous],
                        &mut from_neighbour[0].1,
                        &frames[neighbour],
                        direction,
                        parallel_offset,
                    )
                } else {
                    let (before_previous, from_previous) = bounds.split_at_mut(previous);
                    place_grid_neighbour(
                        &from_previous[0].1,
                        &frames[previous],
                        &mut before_previous[neighbour].1,
                        &frames[neighbour],
                        direction,
                        parallel_offset,
                    )
                };
                placed?;
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

    // Unrelated pairs must not overlap in client space. The physical
    // adjacency edges above are allowed to touch, but never to cover one another.
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

fn place_grid_neighbour(
    from_bounds: &Rect,
    from_frame: &NativeFrameGeometry,
    to_bounds: &mut Rect,
    to_frame: &NativeFrameGeometry,
    direction: WindowGridDirection,
    parallel_offset: Option<f32>,
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
        WindowGridDirection::Above | WindowGridDirection::Below => parallel_offset
            .map(|offset| from_bounds.origin.x + offset)
            .unwrap_or_else(|| {
                from_bounds.origin.x + from_frame.client_left_inset - to_frame.client_left_inset
            }),
    };

    let y = match direction {
        WindowGridDirection::Left | WindowGridDirection::Right => parallel_offset
            .map(|offset| from_bounds.origin.y + offset)
            .unwrap_or(from_bounds.origin.y),
        WindowGridDirection::Below => {
            let from_client_bottom = from_bounds.origin.y
                + from_frame.client_top_inset
                + from_frame.client_height;
            from_client_bottom - to_frame.client_top_inset
        }
        WindowGridDirection::Above => {
            let from_client_top = from_bounds.origin.y + from_frame.client_top_inset;
            from_client_top - to_frame.client_top_inset - to_frame.client_height
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
    mut bounds: Vec<(WindowGridMember, Rect)>,
    target: WindowGridMember,
    moving: WindowGridMember,
    moving_minimum_width: f32,
) -> Option<Vec<(WindowGridMember, Rect)>> {
    let target_index = bounds.iter().position(|(group, _)| *group == target)?;
    let moving_index = bounds.iter().position(|(group, _)| *group == moving)?;
    let target_width = bounds[target_index].1.size.width;
    if !target_width.is_finite()
        || target_width <= 0.0
        || !moving_minimum_width.is_finite()
        || moving_minimum_width <= 0.0
        || target_width < moving_minimum_width
    {
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

/// Returns the change in a resized window's bottom edge in physical pixels.
pub(crate) fn native_resize_bottom_edge_delta(
    previous: Rect,
    actual: Rect,
    scale_factor: f32,
) -> Option<f32> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let previous_bottom = previous.origin.y + previous.size.height * scale_factor;
    let actual_bottom = actual.origin.y + actual.size.height * scale_factor;
    let delta = actual_bottom - previous_bottom;
    delta.is_finite().then_some(delta)
}

fn direction_from_member(
    link: WindowGridLink,
    member: WindowGridMember,
) -> Option<WindowGridDirection> {
    if link.from == member {
        Some(link.direction)
    } else if link.to == member {
        Some(link.direction.opposite())
    } else {
        None
    }
}

fn collect_window_grid_region(
    seeds: &[WindowGridMember],
    resized_group: WindowGridMember,
    adjacency: &HashMap<WindowGridMember, Vec<WindowGridMember>>,
    mut belongs_to_region: impl FnMut(WindowGridMember) -> bool,
) -> HashSet<WindowGridMember> {
    let mut visited = HashSet::new();
    let mut pending = VecDeque::new();

    for seed in seeds {
        if *seed != resized_group && belongs_to_region(*seed) && visited.insert(*seed) {
            pending.push_back(*seed);
        }
    }

    while let Some(current) = pending.pop_front() {
        if let Some(neighbours) = adjacency.get(&current) {
            for neighbour in neighbours {
                if *neighbour != resized_group
                    && !visited.contains(neighbour)
                    && belongs_to_region(*neighbour)
                {
                    visited.insert(*neighbour);
                    pending.push_back(*neighbour);
                }
            }
        }
    }

    visited
}

/// Reflows only the eastern and southern regions connected to the resized
/// window's corresponding edges. Origins are physical desktop pixels; every
/// neighbouring window keeps its own logical size and DPI scale.
///
/// The east region is anchored to the resized HWND's physical client-right edge.
/// Each south branch is anchored to that branch's first lower client top, so a
/// resize of an upper-left window does not move an unrelated lower-right branch.
pub(crate) fn shift_window_grid_after_resize(
    mut bounds: Vec<(WindowGridMember, Rect)>,
    frames: &[NativeFrameGeometry],
    links: &[WindowGridLink],
    resized_group: WindowGridMember,
    right_edge_delta: f32,
    bottom_edge_delta: f32,
) -> Option<Vec<(WindowGridMember, Rect)>> {
    if !right_edge_delta.is_finite()
        || !bottom_edge_delta.is_finite()
        || bounds.len() != frames.len()
    {
        return None;
    }

    let indices = bounds
        .iter()
        .enumerate()
        .map(|(index, (group, rect))| {
            let valid_rect = rect.origin.x.is_finite()
                && rect.origin.y.is_finite()
                && rect.size.width.is_finite()
                && rect.size.width > 0.0
                && rect.size.height.is_finite()
                && rect.size.height > 0.0;
            (valid_rect && frames[index].is_valid())
                .then_some((*group, index))
        })
        .collect::<Option<HashMap<_, _>>>()?;
    if indices.len() != bounds.len() {
        return None;
    }
    let resized_index = *indices.get(&resized_group)?;

    let mut adjacency = HashMap::<WindowGridMember, Vec<WindowGridMember>>::new();
    let mut right_seeds = Vec::new();
    let mut below_seeds = Vec::new();
    for link in links {
        if link.from == link.to
            || !indices.contains_key(&link.from)
            || !indices.contains_key(&link.to)
        {
            return None;
        }
        adjacency.entry(link.from).or_default().push(link.to);
        adjacency.entry(link.to).or_default().push(link.from);

        match direction_from_member(*link, resized_group) {
            Some(WindowGridDirection::Right) => right_seeds.push(
                if link.from == resized_group { link.to } else { link.from },
            ),
            Some(WindowGridDirection::Below) => below_seeds.push(
                if link.from == resized_group { link.to } else { link.from },
            ),
            _ => {}
        }
    }

    let resized_client_right = bounds[resized_index].1.origin.x
        + frames[resized_index].outer_width
        - frames[resized_index].client_right_inset;
    // Neighbours still sit on the pre-resize edge. Recover that edge from the
    // accepted Core boundary delta before selecting the east-side region.
    let previous_client_right = resized_client_right - right_edge_delta;
    if !previous_client_right.is_finite() {
        return None;
    }
    let east_region = collect_window_grid_region(
        &right_seeds,
        resized_group,
        &adjacency,
        |member| {
            let index = indices[&member];
            let client_left = bounds[index].1.origin.x + frames[index].client_left_inset;
            client_left.is_finite() && client_left >= previous_client_right - 1.5
        },
    );

    let mut south_region = HashSet::new();
    for seed in below_seeds {
        let seed_index = *indices.get(&seed)?;
        let branch_top = bounds[seed_index].1.origin.y + frames[seed_index].client_top_inset;
        if !branch_top.is_finite() {
            return None;
        }
        let branch = collect_window_grid_region(
            &[seed],
            resized_group,
            &adjacency,
            |member| {
                let index = indices[&member];
                let client_top = bounds[index].1.origin.y + frames[index].client_top_inset;
                client_top.is_finite() && client_top >= branch_top - 1.5
            },
        );
        south_region.extend(branch);
    }

    for (member, member_bounds) in bounds.iter_mut() {
        if *member == resized_group {
            continue;
        }
        if east_region.contains(member) {
            member_bounds.origin.x += right_edge_delta;
        }
        if south_region.contains(member) {
            member_bounds.origin.y += bottom_edge_delta;
        }
        if !member_bounds.origin.x.is_finite() || !member_bounds.origin.y.is_finite() {
            return None;
        }
    }

    Some(bounds)
}

/// Returns the native Z priority from front to back for one WindowGrid.
///
/// A left neighbor stays in front of its right neighbor so their shared resize
/// edge remains reachable. An upper window stays in front of the lower window,
/// preserving deterministic edge-hit priority without relying on overlap.
pub(crate) fn floating_grid_members_in_native_front_to_back(
    members: &[WindowGridMember],
    links: &[WindowGridLink],
) -> Option<Vec<WindowGridMember>> {
    let member_set = members.iter().copied().collect::<HashSet<_>>();
    if member_set.len() != members.len() {
        return None;
    }

    let mut outgoing = HashMap::<WindowGridMember, Vec<WindowGridMember>>::new();
    let mut indegree = members
        .iter()
        .copied()
        .map(|member| (member, 0usize))
        .collect::<HashMap<_, _>>();

    for link in links {
        let (front, back) = match link.direction {
            WindowGridDirection::Left => (link.to, link.from),
            WindowGridDirection::Right => (link.from, link.to),
            WindowGridDirection::Above => (link.to, link.from),
            WindowGridDirection::Below => (link.from, link.to),
        };
        if front == back || !member_set.contains(&front) || !member_set.contains(&back) {
            return None;
        }

        outgoing.entry(front).or_default().push(back);
        let degree = indegree.get_mut(&back)?;
        *degree = degree.checked_add(1)?;
    }

    // Stable Kahn ordering: when multiple windows have no precedence
    // constraint, preserve the grid's established member order.
    let mut remaining = members.to_vec();
    let mut ordered = Vec::with_capacity(members.len());
    while !remaining.is_empty() {
        let next_index = remaining
            .iter()
            .position(|member| indegree.get(member) == Some(&0))?;
        let front = remaining.remove(next_index);
        ordered.push(front);

        if let Some(neighbours) = outgoing.get(&front) {
            for neighbour in neighbours {
                let degree = indegree.get_mut(neighbour)?;
                *degree = degree.checked_sub(1)?;
            }
        }
    }

    Some(ordered)
}

/// Produces a deterministic back-to-front order while keeping each connected
/// component adjacent in the native Z stack.
pub(crate) fn window_grid_members_in_native_z_order(
    groups: &[(WindowGridMember, i32)],
    grids: &WindowGridRegistry,
) -> Vec<WindowGridMember> {
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

        let links = grids.links_for(*group_id).unwrap_or_default();
        let mut back_to_front =
            floating_grid_members_in_native_front_to_back(&block, links)
                .unwrap_or_else(|| block.clone());
        back_to_front.reverse();

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
        blocks.push((block_z_order, back_to_front));
    }

    blocks.sort_by_key(|(z_order, _)| *z_order);
    blocks
        .into_iter()
        .flat_map(|(_, members)| members)
        .collect()
}

#[cfg(test)]
mod contiguous_window_chain_bounds_tests {
    use super::{NativeFrameGeometry, WindowGridMember, arrange_contiguous_window_chain_bounds};
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::{Key, SlotMap};

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [WindowGridMember; 3]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [
            WindowGridMember::FloatingComposition(ids.insert(())),
            WindowGridMember::FloatingComposition(ids.insert(())),
            WindowGridMember::FloatingComposition(ids.insert(())),
        ];
        (ids, groups)
    }

    fn frame(
        outer_width: f32,
        outer_height: f32,
        visible_left_inset: f32,
        visible_right_inset: f32,
        scale_factor: f32,
    ) -> NativeFrameGeometry {
        NativeFrameGeometry {
            outer_width,
            outer_height,
            visible_left_inset,
            visible_right_inset,
            client_left_inset: 0.0,
            client_top_inset: 0.0,
            client_right_inset: 0.0,
            client_bottom_inset: 0.0,
            client_width: outer_width,
            client_height: outer_height,
            scale_factor,
        }
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
            arrange_contiguous_window_chain_bounds(input, &frames, WindowGridMember::FloatingComposition(FloatingGroupId::null()))
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
            frame(450.0, 360.0, 6.0, 6.0, 1.5),
            frame(500.0, 480.0, 8.0, 8.0, 1.25),
            frame(400.0, 300.0, 4.0, 4.0, 2.0),
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
                    frame(100.0, 200.0, 50.0, 50.0, 1.0),
                ],
                a,
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod window_grid_bounds_tests {
    use super::{
        NativeFrameGeometry, arrange_window_grid_bounds,
        arrange_window_grid_bounds_with_preserved_offsets, floating_column_outer_bounds,
        match_moving_window_width_to_target,
    };
    use crate::window_chain::{WindowGridDirection, WindowGridLink, WindowGridMember};
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups(count: usize) -> Vec<WindowGridMember> {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        (0..count)
            .map(|_| WindowGridMember::FloatingComposition(ids.insert(())))
            .collect()
    }

    fn bounds(
        id: WindowGridMember,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> (WindowGridMember, Rect) {
        (id, Rect::new(Point::new(x, y), Size::new(width, height)))
    }

    fn frame(logical_width: f32, logical_height: f32, scale: f32) -> NativeFrameGeometry {
        NativeFrameGeometry::without_frame_size(
            Size::new(logical_width, logical_height),
            scale,
        )
        .unwrap()
    }

    fn link(from: WindowGridMember, to: WindowGridMember, direction: WindowGridDirection) -> WindowGridLink {
        WindowGridLink { from, to, direction }
    }

    fn frame_with_native_insets(
        logical_width: f32,
        logical_height: f32,
        scale_factor: f32,
        visible_left_inset: f32,
        visible_right_inset: f32,
        client_left_inset: f32,
        client_top_inset: f32,
        client_right_inset: f32,
        client_bottom_inset: f32,
    ) -> NativeFrameGeometry {
        let client_width = logical_width * scale_factor;
        let client_height = logical_height * scale_factor;
        NativeFrameGeometry {
            outer_width: client_left_inset + client_width + client_right_inset,
            outer_height: client_top_inset + client_height + client_bottom_inset,
            visible_left_inset,
            visible_right_inset,
            client_left_inset,
            client_top_inset,
            client_right_inset,
            client_bottom_inset,
            client_width,
            client_height,
            scale_factor,
        }
    }

    #[test]
    fn floating_column_bounds_keep_header_and_groups_on_the_core_client_grid() {
        let frame = frame_with_native_insets(
            200.0, 120.0, 1.5, 6.0, 6.0, 8.0, 30.0, 8.0, 10.0,
        );
        let floating_origin = Point::new(1000.0, 600.0);

        let header = floating_column_outer_bounds(
            floating_origin,
            Point::ZERO,
            Size::new(240.0, 40.0),
            frame,
        )
        .expect("Header should align to the original Floating outer origin");
        let group = floating_column_outer_bounds(
            floating_origin,
            Point::new(0.0, 40.0),
            Size::new(240.0, 180.0),
            frame,
        )
        .expect("Group should align below the Header using Core logical coordinates");

        assert_eq!(header.origin, floating_origin);
        assert_eq!(group.origin, Point::new(1000.0, 660.0));
        assert_eq!(header.size.width, group.size.width);
        assert_eq!(
            group.origin.y - header.origin.y,
            40.0 * frame.scale_factor,
        );
    }

    #[test]
    fn outer_window_bounds_preserve_a_client_rect_at_fractional_dpi() {
        let frame = frame_with_native_insets(
            200.0, 120.0, 1.5, 6.0, 6.0, 8.0, 30.0, 8.0, 10.0,
        );
        let client = Rect::new(Point::new(1000.0, 600.0), Size::new(240.0, 180.0));

        let outer = frame
            .outer_bounds_for_client_rect(client)
            .expect("valid client geometry and native insets");

        assert_eq!(outer.origin, Point::new(992.0, 570.0));
        assert_eq!(outer.size, client.size);
        assert_eq!(
            Point::new(
                outer.origin.x + frame.client_left_inset,
                outer.origin.y + frame.client_top_inset,
            ),
            client.origin,
        );
    }

    #[test]
    fn native_frame_measurements_capture_client_insets_and_monitor_scale() {
        use platform::{PhysicalPoint, PhysicalRect, PhysicalSize};

        let frame = NativeFrameGeometry::from_native_rects(
            PhysicalRect {
                left: 100,
                top: 200,
                right: 640,
                bottom: 720,
            },
            PhysicalRect {
                left: 106,
                top: 200,
                right: 634,
                bottom: 720,
            },
            PhysicalPoint { x: 108, y: 230 },
            PhysicalSize {
                width: 524,
                height: 480,
            },
            1.5,
        )
        .expect("consistent native outer/client rectangles");

        assert_eq!(frame.outer_width, 540.0);
        assert_eq!(frame.outer_height, 520.0);
        assert_eq!(frame.visible_left_inset, 6.0);
        assert_eq!(frame.visible_right_inset, 6.0);
        assert_eq!(frame.client_left_inset, 8.0);
        assert_eq!(frame.client_top_inset, 30.0);
        assert_eq!(frame.client_right_inset, 8.0);
        assert_eq!(frame.client_bottom_inset, 10.0);
        assert_eq!(frame.client_width, 524.0);
        assert_eq!(frame.client_height, 480.0);
        assert_eq!(frame.scale_factor, 1.5);
    }

    #[test]
    fn planned_client_size_uses_each_hwnd_scale_and_preserves_frame_insets() {
        use platform::{PhysicalPoint, PhysicalRect, PhysicalSize};

        let measured = NativeFrameGeometry::from_native_rects(
            PhysicalRect {
                left: 100,
                top: 200,
                right: 640,
                bottom: 720,
            },
            PhysicalRect {
                left: 106,
                top: 200,
                right: 634,
                bottom: 720,
            },
            PhysicalPoint { x: 108, y: 230 },
            PhysicalSize {
                width: 524,
                height: 480,
            },
            1.5,
        )
        .expect("consistent native frame");

        let planned = measured
            .with_logical_client_size(Size::new(240.0, 300.0))
            .expect("valid planned client size");
        assert_eq!(planned.client_width, 360.0);
        assert_eq!(planned.client_height, 450.0);
        assert_eq!(planned.outer_width, 376.0);
        assert_eq!(planned.outer_height, 490.0);
        assert_eq!(planned.visible_left_inset, 6.0);
        assert_eq!(planned.visible_right_inset, 6.0);
        assert_eq!(planned.client_left_inset, 8.0);
        assert_eq!(planned.client_top_inset, 30.0);
        assert_eq!(planned.client_right_inset, 8.0);
        assert_eq!(planned.client_bottom_inset, 10.0);
    }

    #[test]
    fn all_join_directions_respect_fractional_dpi_and_native_frame_insets() {
        let [anchor, moving] = groups(2).try_into().expect("two group ids");
        let anchor_bounds = bounds(anchor, 100.0, 200.0, 200.0, 120.0);
        let moving_bounds = bounds(moving, 900.0, 1000.0, 160.0, 100.0);
        let anchor_frame = frame_with_native_insets(
            200.0, 120.0, 1.25, 2.0, 3.0, 8.0, 10.0, 7.0, 6.0,
        );
        let moving_frame = frame_with_native_insets(
            160.0, 100.0, 1.5, 4.0, 5.0, 13.0, 17.0, 8.0, 9.0,
        );

        for direction in [
            WindowGridDirection::Left,
            WindowGridDirection::Right,
            WindowGridDirection::Above,
            WindowGridDirection::Below,
        ] {
            let input = vec![anchor_bounds, moving_bounds];
            let arranged = arrange_window_grid_bounds(
                input,
                &[anchor_frame, moving_frame],
                &[link(anchor, moving, direction)],
                anchor,
            )
            .expect("fractional-DPI native frames should produce a valid join");

            assert_eq!(arranged[0], anchor_bounds, "{direction:?} moved its anchor");
            assert_eq!(arranged[1].1.size, moving_bounds.1.size);

            let expected_x = match direction {
                WindowGridDirection::Left => {
                    anchor_bounds.1.origin.x
                        + anchor_frame.visible_left_inset
                        - moving_frame.outer_width
                        + moving_frame.visible_right_inset
                }
                WindowGridDirection::Right => {
                    anchor_bounds.1.origin.x
                        + anchor_frame.outer_width
                        - anchor_frame.visible_right_inset
                        - moving_frame.visible_left_inset
                }
                WindowGridDirection::Above | WindowGridDirection::Below => {
                    anchor_bounds.1.origin.x
                        + anchor_frame.client_left_inset
                        - moving_frame.client_left_inset
                }
            };
            let expected_y = match direction {
                WindowGridDirection::Left | WindowGridDirection::Right => {
                    anchor_bounds.1.origin.y
                }
                WindowGridDirection::Below => {
                    anchor_bounds.1.origin.y
                        + anchor_frame.client_top_inset
                        + anchor_frame.client_height
                        - moving_frame.client_top_inset
                }
                WindowGridDirection::Above => {
                    anchor_bounds.1.origin.y
                        + anchor_frame.client_top_inset
                        - moving_frame.client_top_inset
                        - moving_frame.client_height
                }
            };
            assert_eq!(
                arranged[1].1.origin,
                Point::new(expected_x, expected_y),
                "{direction:?} used the wrong physical edge or monitor scale",
            );
        }
    }

    #[test]
    fn vertical_edges_touch_client_bounds_with_mixed_dpi_and_frame_insets() {
        let [anchor, moving] = groups(2).try_into().expect("two group ids");
        let input = vec![
            bounds(anchor, 100.0, 200.0, 200.0, 120.0),
            bounds(moving, 900.0, 1000.0, 160.0, 100.0),
        ];
        let anchor_frame = frame_with_native_insets(
            200.0, 120.0, 1.25, 2.0, 3.0, 8.0, 10.0, 7.0, 6.0,
        );
        let moving_frame = frame_with_native_insets(
            160.0, 100.0, 1.5, 4.0, 5.0, 13.0, 17.0, 8.0, 9.0,
        );
        let frames = [anchor_frame, moving_frame];

        let below = arrange_window_grid_bounds(
            input.clone(),
            &frames,
            &[link(anchor, moving, WindowGridDirection::Below)],
            anchor,
        )
        .expect("a lower window should align at the client edge");
        let anchor_client_bottom =
            below[0].1.origin.y + anchor_frame.client_top_inset + anchor_frame.client_height;
        let moving_client_top = below[1].1.origin.y + moving_frame.client_top_inset;
        assert!((anchor_client_bottom - moving_client_top).abs() < 0.001);
        assert_eq!(
            below[0].1.origin.x + anchor_frame.client_left_inset,
            below[1].1.origin.x + moving_frame.client_left_inset,
        );

        let above = arrange_window_grid_bounds(
            input,
            &frames,
            &[link(anchor, moving, WindowGridDirection::Above)],
            anchor,
        )
        .expect("an upper window should align at the client edge");
        let moving_client_bottom =
            above[1].1.origin.y + moving_frame.client_top_inset + moving_frame.client_height;
        let anchor_client_top = above[0].1.origin.y + anchor_frame.client_top_inset;
        assert!((moving_client_bottom - anchor_client_top).abs() < 0.001);
        assert_eq!(
            above[0].1.origin.x + anchor_frame.client_left_inset,
            above[1].1.origin.x + moving_frame.client_left_inset,
        );
    }

    #[test]
    fn internal_horizontal_links_preserve_column_vertical_offsets_during_alignment() {
        use std::collections::HashSet;

        let [anchor, internal, external] =
            groups(3).try_into().expect("three group ids");
        let initial = vec![
            bounds(anchor, 100.0, 200.0, 100.0, 80.0),
            bounds(internal, 208.0, 245.0, 100.0, 160.0),
            bounds(external, 600.0, 320.0, 100.0, 120.0),
        ];
        let internal_link = link(anchor, internal, WindowGridDirection::Right);
        let external_link = link(internal, external, WindowGridDirection::Right);
        let frames = [
            frame(100.0, 80.0, 1.0),
            frame(100.0, 160.0, 1.0),
            frame(100.0, 120.0, 1.0),
        ];

        let arranged = arrange_window_grid_bounds_with_preserved_offsets(
            initial,
            &frames,
            &[internal_link, external_link],
            anchor,
            &HashSet::from([internal_link]),
        )
        .expect("internal Column offsets should coexist with an external horizontal join");

        assert_eq!(arranged[0].1.origin, Point::new(100.0, 200.0));
        assert_eq!(arranged[1].1.origin, Point::new(200.0, 245.0));
        assert_eq!(arranged[2].1.origin, Point::new(300.0, 245.0));
    }

    #[test]
    fn malformed_grid_geometry_is_rejected_without_partial_results() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let original = vec![
            bounds(a, 10.0, 20.0, 100.0, 80.0),
            bounds(b, 300.0, 20.0, 100.0, 80.0),
            bounds(c, 600.0, 20.0, 100.0, 80.0),
        ];
        let frames = [
            frame(100.0, 80.0, 1.0),
            frame(100.0, 80.0, 1.0),
            frame(100.0, 80.0, 1.0),
        ];

        assert!(
            arrange_window_grid_bounds(
                vec![original[0].clone(), original[0].clone()],
                &[frames[0], frames[0]],
                &[],
                a,
            )
            .is_none(),
            "duplicate FloatingGroup IDs must be rejected",
        );
        assert!(
            arrange_window_grid_bounds(
                original.clone(),
                &frames,
                &[link(a, b, WindowGridDirection::Right)],
                a,
            )
            .is_none(),
            "disconnected components must not be laid out as one grid",
        );
        assert!(
            arrange_window_grid_bounds(
                original.clone(),
                &frames,
                &[
                    link(a, b, WindowGridDirection::Right),
                    link(b, c, WindowGridDirection::Right),
                    link(a, c, WindowGridDirection::Below),
                ],
                a,
            )
            .is_none(),
            "extra cyclic edges must be rejected",
        );
        assert_eq!(original, vec![
            bounds(a, 10.0, 20.0, 100.0, 80.0),
            bounds(b, 300.0, 20.0, 100.0, 80.0),
            bounds(c, 600.0, 20.0, 100.0, 80.0),
        ]);
    }

    #[test]
    fn places_a_vertical_stack_without_overlap_and_preserves_anchor() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let input = vec![
            bounds(a, 100.0, 100.0, 300.0, 300.0),
            bounds(b, 900.0, 20.0, 300.0, 200.0),
            bounds(c, 1500.0, 40.0, 300.0, 100.0),
        ];
        let frames = [
            frame(300.0, 300.0, 1.0),
            frame(300.0, 200.0, 1.5),
            frame(300.0, 100.0, 1.0),
        ];
        let links = [
            link(a, b, WindowGridDirection::Below),
            link(b, c, WindowGridDirection::Below),
        ];

        let arranged = arrange_window_grid_bounds(input, &frames, &links, a)
            .expect("a vertical grid should have finite bounds");

        assert_eq!(arranged[0].1.origin, Point::new(100.0, 100.0));
        let second_y = 100.0 + frames[0].client_height - frames[1].client_top_inset;
        let third_y = second_y + frames[1].client_height - frames[2].client_top_inset;
        assert_eq!(arranged[1].1.origin, Point::new(100.0, second_y));
        assert_eq!(arranged[2].1.origin, Point::new(100.0, third_y));
        assert_eq!(arranged[0].1.size, Size::new(300.0, 300.0));
        assert_eq!(arranged[1].1.size, Size::new(300.0, 200.0));
        assert_eq!(arranged[2].1.size, Size::new(300.0, 100.0));

        let a_client_bottom = arranged[0].1.origin.y + frames[0].client_height;
        let b_client_top = arranged[1].1.origin.y + frames[1].client_top_inset;
        assert_eq!(a_client_bottom, b_client_top);

        let b_client_bottom = arranged[1].1.origin.y + frames[1].client_height;
        let c_client_top = arranged[2].1.origin.y + frames[2].client_top_inset;
        assert_eq!(b_client_bottom, c_client_top);
    }

    #[test]
    fn places_a_window_above_the_target_and_keeps_target_fixed() {
        let [moving, target] = groups(2).try_into().expect("two group ids");
        let input = vec![
            bounds(moving, 700.0, 900.0, 300.0, 200.0),
            bounds(target, 120.0, 500.0, 300.0, 300.0),
        ];
        let frames = [frame(300.0, 200.0, 1.5), frame(300.0, 300.0, 1.0)];
        let links = [link(target, moving, WindowGridDirection::Above)];

        let arranged = arrange_window_grid_bounds(input, &frames, &links, target)
            .expect("the upper window should be positioned");

        assert_eq!(arranged[1].1.origin, Point::new(120.0, 500.0));
        assert_eq!(arranged[0].1.origin, Point::new(120.0, 200.0));
        let moving_client_bottom =
            arranged[0].1.origin.y + frames[0].client_top_inset + frames[0].client_height;
        let target_client_top = arranged[1].1.origin.y + frames[1].client_top_inset;
        assert_eq!(moving_client_bottom, target_client_top);
    }

    #[test]
    fn keeps_the_existing_horizontal_join_formula_unchanged() {
        let [a, b] = groups(2).try_into().expect("two group ids");
        let input = vec![
            bounds(a, 10.0, 70.0, 200.0, 120.0),
            bounds(b, 800.0, 20.0, 150.0, 120.0),
        ];
        let frames = [frame(200.0, 120.0, 1.0), frame(150.0, 120.0, 1.0)];
        let links = [link(a, b, WindowGridDirection::Right)];

        let arranged = arrange_window_grid_bounds(input, &frames, &links, a)
            .expect("the horizontal grid should be positioned");

        assert_eq!(arranged[0].1.origin, Point::new(10.0, 70.0));
        assert_eq!(arranged[1].1.origin, Point::new(210.0, 70.0));
    }

    #[test]
    fn supports_a_t_shaped_grid_without_unrelated_client_overlap() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let input = vec![
            bounds(a, 100.0, 100.0, 200.0, 200.0),
            bounds(b, 700.0, 500.0, 200.0, 100.0),
            bounds(c, 900.0, 800.0, 100.0, 200.0),
            bounds(d, 1200.0, 900.0, 100.0, 100.0),
        ];
        let frames = [
            frame(200.0, 200.0, 1.0),
            frame(200.0, 100.0, 1.0),
            frame(100.0, 200.0, 1.0),
            frame(100.0, 100.0, 1.0),
        ];
        let links = [
            link(a, b, WindowGridDirection::Above),
            link(a, c, WindowGridDirection::Right),
            link(c, d, WindowGridDirection::Below),
        ];

        let arranged = arrange_window_grid_bounds(input, &frames, &links, a)
            .expect("a T-shaped grid should have a consistent embedding");

        assert_eq!(arranged[0].1.origin, Point::new(100.0, 100.0));
        assert_eq!(arranged[1].1.origin, Point::new(100.0, 0.0));
        assert_eq!(arranged[2].1.origin, Point::new(300.0, 100.0));
        assert_eq!(
            arranged[3].1.origin,
            Point::new(300.0, 300.0)
        );
    }

    #[test]
    fn repeated_same_side_drops_extend_the_lane_without_overlapping() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let input = vec![
            bounds(a, 100.0, 100.0, 300.0, 300.0),
            bounds(b, 900.0, 0.0, 300.0, 200.0),
            bounds(c, 1500.0, 0.0, 300.0, 150.0),
        ];
        let frames = [
            frame(300.0, 300.0, 1.0),
            frame(300.0, 200.0, 1.0),
            frame(300.0, 150.0, 1.0),
        ];
        let links = [
            link(a, b, WindowGridDirection::Below),
            link(a, c, WindowGridDirection::Below),
        ];

        let arranged = arrange_window_grid_bounds(input, &frames, &links, a)
            .expect("same-side neighbors should form a vertical lane");

        let second_y = 100.0 + frames[0].client_height;
        let third_y = second_y + frames[1].client_height;
        assert_eq!(arranged[1].1.origin.y, second_y);
        assert_eq!(arranged[2].1.origin.y, third_y);
    }

    #[test]
    fn rejects_conflicting_branches_that_would_overlap_unrelated_windows() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let input = vec![
            bounds(a, 100.0, 100.0, 300.0, 300.0),
            bounds(b, 900.0, 0.0, 300.0, 200.0),
            bounds(c, 1500.0, 0.0, 300.0, 150.0),
            bounds(d, 1800.0, 0.0, 300.0, 100.0),
        ];
        let frames = [
            frame(300.0, 300.0, 1.0),
            frame(300.0, 200.0, 1.0),
            frame(300.0, 150.0, 1.0),
            frame(300.0, 100.0, 1.0),
        ];
        let links = [
            link(a, b, WindowGridDirection::Below),
            link(a, c, WindowGridDirection::Below),
            link(b, d, WindowGridDirection::Below),
        ];

        assert!(
            arrange_window_grid_bounds(input, &frames, &links, a).is_none(),
            "a conflicting branch must not commit overlapping windows"
        );
    }

    #[test]
    fn arrangement_is_idempotent_for_the_same_grid_and_anchor() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let input = vec![
            bounds(a, 100.0, 100.0, 300.0, 300.0),
            bounds(b, 900.0, 0.0, 300.0, 200.0),
            bounds(c, 1500.0, 0.0, 300.0, 150.0),
        ];
        let frames = [
            frame(300.0, 300.0, 1.0),
            frame(300.0, 200.0, 1.0),
            frame(300.0, 150.0, 1.0),
        ];
        let links = [
            link(a, b, WindowGridDirection::Below),
            link(b, c, WindowGridDirection::Below),
        ];
        let first =
            arrange_window_grid_bounds(input, &frames, &links, a).unwrap();
        let second =
            arrange_window_grid_bounds(first.clone(), &frames, &links, a).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn moving_width_uses_the_target_logical_width_without_resizing_target() {
        let [target, moving] = groups(2).try_into().expect("two group ids");
        let target_bounds = Rect::new(Point::new(20.0, 30.0), Size::new(240.0, 300.0));
        let moving_bounds = Rect::new(Point::new(800.0, 50.0), Size::new(420.0, 500.0));
        let result = match_moving_window_width_to_target(
            vec![(target, target_bounds), (moving, moving_bounds)],
            target,
            moving,
            120.0,
        )
        .expect("both windows must exist");

        assert_eq!(result[0].1, target_bounds);
        assert_eq!(result[1].1.size.width, 240.0);
        assert_eq!(result[1].1.size.height, moving_bounds.size.height);
    }

    #[test]
    fn moving_width_normalization_rejects_a_target_below_the_moving_minimum() {
        let [target, moving] = groups(2).try_into().expect("two group ids");
        let target_bounds = Rect::new(Point::new(20.0, 30.0), Size::new(240.0, 300.0));
        let moving_bounds = Rect::new(Point::new(800.0, 50.0), Size::new(420.0, 500.0));

        assert!(
            match_moving_window_width_to_target(
                vec![(target, target_bounds), (moving, moving_bounds)],
                target,
                moving,
                280.0,
            )
            .is_none(),
            "the target width must not be expanded to satisfy the moving window"
        );
    }
}

#[cfg(test)]
mod native_resize_grid_geometry_tests {
    use super::{
        NativeFrameGeometry, native_resize_bottom_edge_delta, native_resize_right_edge_delta,
        shift_window_grid_after_resize,
    };
    use crate::window_chain::{WindowGridDirection, WindowGridLink, WindowGridMember};
    use dock_core::{FloatingGroupId, NodeId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups(count: usize) -> (SlotMap<FloatingGroupId, ()>, Vec<WindowGridMember>) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = (0..count)
            .map(|_| WindowGridMember::FloatingComposition(ids.insert(())))
            .collect();
        (ids, groups)
    }

    fn columns(count: usize) -> (SlotMap<NodeId, ()>, Vec<WindowGridMember>) {
        let mut ids = SlotMap::<NodeId, ()>::with_key();
        let columns = (0..count)
            .map(|_| WindowGridMember::Column(ids.insert(())))
            .collect();
        (ids, columns)
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    fn frame(width: f32, height: f32, scale: f32) -> NativeFrameGeometry {
        NativeFrameGeometry::without_frame_size(Size::new(width, height), scale)
            .expect("valid frame geometry")
    }

    fn frame_with_insets(
        width: f32,
        height: f32,
        client_left: f32,
        client_right: f32,
        client_top: f32,
        client_bottom: f32,
        visible_left: f32,
        visible_right: f32,
    ) -> NativeFrameGeometry {
        NativeFrameGeometry {
            outer_width: client_left + width + client_right,
            outer_height: client_top + height + client_bottom,
            visible_left_inset: visible_left,
            visible_right_inset: visible_right,
            client_left_inset: client_left,
            client_top_inset: client_top,
            client_right_inset: client_right,
            client_bottom_inset: client_bottom,
            client_width: width,
            client_height: height,
            scale_factor: 1.0,
        }
    }

    fn link(
        from: WindowGridMember,
        to: WindowGridMember,
        direction: WindowGridDirection,
    ) -> WindowGridLink {
        WindowGridLink { from, to, direction }
    }

    #[test]
    fn column_window_grid_members_reflow_eastern_and_southern_branches() {
        let (_ids, members) = columns(3);
        let [resized, east, south] = members.as_slice() else {
            panic!("three Column window members");
        };
        let bounds = vec![
            (*resized, rect(0.0, 0.0, 124.0, 80.0)),
            (*east, rect(124.0, 0.0, 50.0, 40.0)),
            (*south, rect(0.0, 80.0, 100.0, 40.0)),
        ];
        let frames = vec![
            frame(124.0, 80.0, 1.0),
            frame(50.0, 40.0, 1.0),
            frame(100.0, 40.0, 1.0),
        ];
        let links = vec![
            link(*resized, *east, WindowGridDirection::Right),
            link(*resized, *south, WindowGridDirection::Below),
        ];

        let shifted = shift_window_grid_after_resize(
            bounds,
            &frames,
            &links,
            *resized,
            24.0,
            12.0,
        )
        .expect("valid Column WindowGrid");

        assert_eq!(shifted[0].1.origin, Point::new(0.0, 0.0));
        assert_eq!(shifted[1].1.origin, Point::new(148.0, 0.0));
        assert_eq!(shifted[2].1.origin, Point::new(0.0, 92.0));
    }

    #[test]
    fn right_edge_resize_moves_only_the_eastern_connected_region() {
        let (_ids, ids) = groups(4);
        let [a, b, c, d] = ids.as_slice() else { panic!("four groups"); };
        let bounds = vec![
            (*a, rect(0.0, 0.0, 124.0, 80.0)),
            (*b, rect(100.0, 0.0, 50.0, 40.0)),
            (*c, rect(0.0, 70.0, 100.0, 40.0)),
            (*d, rect(100.0, 30.0, 50.0, 50.0)),
        ];
        let frames = vec![
            frame(124.0, 80.0, 1.0),
            frame(50.0, 40.0, 1.0),
            frame(100.0, 40.0, 1.0),
            frame(50.0, 50.0, 1.0),
        ];
        let links = vec![
            link(*a, *b, WindowGridDirection::Right),
            link(*a, *c, WindowGridDirection::Below),
            link(*b, *d, WindowGridDirection::Below),
        ];

        let shifted = shift_window_grid_after_resize(bounds, &frames, &links, *a, 24.0, 0.0)
            .expect("valid right-side grid region");
        assert_eq!(shifted[0].1.origin, Point::new(0.0, 0.0));
        assert_eq!(shifted[1].1.origin, Point::new(124.0, 0.0));
        assert_eq!(shifted[2].1.origin, Point::new(0.0, 70.0));
        assert_eq!(shifted[3].1.origin, Point::new(124.0, 30.0));
        assert_eq!(shifted[3].1.size, Size::new(50.0, 50.0));
    }

    #[test]
    fn east_branch_uses_client_edges_when_visible_frame_insets_differ() {
        let (_ids, ids) = groups(4);
        let [a, b, c, d] = ids.as_slice() else { panic!("four groups"); };
        let bounds = vec![
            (*a, rect(0.0, 0.0, 120.0, 80.0)),
            (*b, rect(110.0, 0.0, 50.0, 40.0)),
            (*c, rect(-2.0, 70.0, 100.0, 40.0)),
            (*d, rect(110.0, 35.0, 50.0, 50.0)),
        ];
        let frames = vec![
            frame_with_insets(120.0, 80.0, 10.0, 10.0, 5.0, 5.0, 2.0, 4.0),
            frame_with_insets(50.0, 40.0, 14.0, 6.0, 5.0, 5.0, 6.0, 4.0),
            frame_with_insets(100.0, 40.0, 12.0, 8.0, 5.0, 5.0, 2.0, 4.0),
            frame_with_insets(50.0, 50.0, 14.0, 6.0, 5.0, 5.0, 1.0, 4.0),
        ];
        let links = vec![
            link(*a, *b, WindowGridDirection::Right),
            link(*a, *c, WindowGridDirection::Below),
            link(*b, *d, WindowGridDirection::Below),
        ];

        let shifted = shift_window_grid_after_resize(bounds, &frames, &links, *a, 20.0, 0.0)
            .expect("valid grid with different frame insets");
        assert_eq!(shifted[1].1.origin.x, 130.0);
        assert_eq!(shifted[2].1.origin.x, -2.0);
        assert_eq!(shifted[3].1.origin.x, 130.0);
    }

    #[test]
    fn bottom_edge_resize_moves_only_the_southern_branch_and_its_row() {
        let (_ids, ids) = groups(5);
        let [a, b, c, d, e] = ids.as_slice() else { panic!("five groups"); };
        let bounds = vec![
            (*a, rect(0.0, 0.0, 100.0, 80.0)),
            (*b, rect(100.0, 0.0, 50.0, 80.0)),
            (*c, rect(0.0, 70.0, 100.0, 40.0)),
            (*d, rect(100.0, 70.0, 50.0, 60.0)),
            (*e, rect(100.0, 120.0, 50.0, 40.0)),
        ];
        let frames = vec![
            frame(100.0, 80.0, 1.0),
            frame(50.0, 80.0, 1.0),
            frame(100.0, 40.0, 1.0),
            frame(50.0, 60.0, 1.0),
            frame(50.0, 40.0, 1.0),
        ];
        let links = vec![
            link(*a, *b, WindowGridDirection::Right),
            link(*a, *c, WindowGridDirection::Below),
            link(*c, *d, WindowGridDirection::Right),
            link(*d, *e, WindowGridDirection::Below),
        ];

        let shifted = shift_window_grid_after_resize(bounds, &frames, &links, *a, 0.0, 18.0)
            .expect("valid lower grid region");
        assert_eq!(shifted[0].1.origin, Point::new(0.0, 0.0));
        assert_eq!(shifted[1].1.origin, Point::new(100.0, 0.0));
        assert_eq!(shifted[2].1.origin, Point::new(0.0, 88.0));
        assert_eq!(shifted[3].1.origin, Point::new(100.0, 88.0));
        assert_eq!(shifted[4].1.origin, Point::new(100.0, 138.0));
        assert_eq!(shifted[4].1.size, Size::new(50.0, 40.0));
    }

    #[test]
    fn corner_resize_combines_axis_offsets_without_changing_sizes() {
        let (_ids, ids) = groups(5);
        let [a, b, c, d, e] = ids.as_slice() else { panic!("five groups"); };
        let bounds = vec![
            (*a, rect(0.0, 0.0, 112.0, 100.0)),
            (*b, rect(100.0, 0.0, 50.0, 40.0)),
            (*c, rect(0.0, 70.0, 100.0, 40.0)),
            (*d, rect(100.0, 30.0, 50.0, 50.0)),
            (*e, rect(0.0, 120.0, 100.0, 40.0)),
        ];
        let frames = vec![
            frame(112.0, 100.0, 1.0),
            frame(50.0, 40.0, 1.0),
            frame(100.0, 40.0, 1.0),
            frame(50.0, 50.0, 1.0),
            frame(100.0, 40.0, 1.0),
        ];
        let links = vec![
            link(*a, *b, WindowGridDirection::Right),
            link(*a, *c, WindowGridDirection::Below),
            link(*b, *d, WindowGridDirection::Below),
            link(*c, *e, WindowGridDirection::Below),
        ];

        let shifted = shift_window_grid_after_resize(bounds, &frames, &links, *a, 12.0, 20.0)
            .expect("valid corner resize regions");
        assert_eq!(shifted[1].1.origin, Point::new(112.0, 0.0));
        assert_eq!(shifted[2].1.origin, Point::new(0.0, 90.0));
        assert_eq!(shifted[3].1.origin, Point::new(112.0, 30.0));
        assert_eq!(shifted[4].1.origin, Point::new(0.0, 140.0));
        assert_eq!(shifted[3].1.size, Size::new(50.0, 50.0));
    }

    #[test]
    fn resizing_middle_window_moves_only_the_eastern_members() {
        let (_ids, ids) = groups(3);
        let [a, b, c] = ids.as_slice() else { panic!("three groups"); };
        let bounds = vec![
            (*a, rect(0.0, 0.0, 100.0, 80.0)),
            (*b, rect(100.0, 0.0, 70.0, 40.0)),
            (*c, rect(150.0, 0.0, 80.0, 40.0)),
        ];
        let frames = vec![
            frame(100.0, 80.0, 1.0),
            frame(70.0, 40.0, 1.0),
            frame(80.0, 40.0, 1.0),
        ];
        let links = vec![
            link(*a, *b, WindowGridDirection::Right),
            link(*b, *c, WindowGridDirection::Right),
        ];

        let shifted = shift_window_grid_after_resize(bounds, &frames, &links, *b, 20.0, 0.0)
            .expect("valid middle member resize");
        assert_eq!(shifted[0].1.origin, Point::new(0.0, 0.0));
        assert_eq!(shifted[1].1.origin, Point::new(100.0, 0.0));
        assert_eq!(shifted[2].1.origin, Point::new(170.0, 0.0));
        assert_eq!(shifted[2].1.size, Size::new(80.0, 40.0));
    }

    #[test]
    fn resizing_terminal_window_does_not_move_any_other_member() {
        let (_ids, ids) = groups(2);
        let [a, b] = ids.as_slice() else { panic!("two groups"); };
        let bounds = vec![
            (*a, rect(0.0, 0.0, 100.0, 80.0)),
            (*b, rect(100.0, 0.0, 50.0, 40.0)),
        ];
        let frames = vec![frame(100.0, 80.0, 1.0), frame(50.0, 40.0, 1.0)];
        let links = vec![link(*a, *b, WindowGridDirection::Right)];

        let shifted = shift_window_grid_after_resize(
            bounds.clone(),
            &frames,
            &links,
            *b,
            40.0,
            25.0,
        )
        .expect("terminal member remains a valid grid");
        assert_eq!(shifted, bounds);
    }

    #[test]
    fn mixed_dpi_grid_members_shift_by_the_same_physical_delta() {
        let (_ids, ids) = groups(3);
        let [a, b, c] = ids.as_slice() else { panic!("three groups"); };
        let bounds = vec![
            (*a, rect(0.0, 0.0, 115.0, 80.0)),
            (*b, rect(100.0, 0.0, 50.0, 40.0)),
            (*c, rect(200.0, 0.0, 30.0, 40.0)),
        ];
        let frames = vec![
            frame(115.0, 80.0, 1.0),
            frame(50.0, 40.0, 2.0),
            frame(30.0, 40.0, 1.0),
        ];
        let links = vec![
            link(*a, *b, WindowGridDirection::Right),
            link(*b, *c, WindowGridDirection::Right),
        ];

        let shifted = shift_window_grid_after_resize(bounds, &frames, &links, *a, 15.0, 0.0)
            .expect("mixed-DPI grid must remain valid");
        assert_eq!(shifted[1].1.origin.x, 115.0);
        assert_eq!(shifted[2].1.origin.x, 215.0);
        assert_eq!(shifted[1].1.size, Size::new(50.0, 40.0));
    }

    #[test]
    fn right_and_bottom_edge_deltas_use_the_accepted_physical_size() {
        let before = rect(100.0, 50.0, 300.0, 200.0);
        let actual = rect(100.0, 50.0, 340.0, 230.0);
        assert_eq!(native_resize_right_edge_delta(before, actual, 1.5), Some(60.0));
        assert_eq!(native_resize_bottom_edge_delta(before, actual, 1.5), Some(45.0));

        let left_top = rect(160.0, 70.0, 260.0, 280.0 / 1.5);
        assert_eq!(native_resize_right_edge_delta(before, left_top, 1.5), Some(0.0));
        assert!(
            native_resize_bottom_edge_delta(before, left_top, 1.5)
                .expect("valid scale")
                .abs()
                < 0.01
        );
    }
}

#[cfg(test)]
mod native_z_order_tests {
    use super::{
        floating_grid_members_in_native_front_to_back,
        window_grid_members_in_native_z_order,
    };
    use crate::window_chain::{WindowGridDirection, WindowGridMember, WindowGridRegistry};
    use dock_core::FloatingGroupId;
    use slotmap::SlotMap;

    #[test]
    fn each_join_direction_places_the_covering_or_edge_owner_in_front() {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let [a, b] = [WindowGridMember::FloatingComposition(ids.insert(())), WindowGridMember::FloatingComposition(ids.insert(()))];
        let cases = [
            (WindowGridDirection::Left, vec![b, a]),
            (WindowGridDirection::Right, vec![a, b]),
            (WindowGridDirection::Above, vec![b, a]),
            (WindowGridDirection::Below, vec![a, b]),
        ];

        for (direction, expected_front_to_back) in cases {
            let mut grids = WindowGridRegistry::default();
            assert!(grids.link_to_side(a, b, direction));
            let members = grids.members_for(a);
            let links = grids.links_for(a).expect("a joined grid has links");

            assert_eq!(
                floating_grid_members_in_native_front_to_back(&members, links),
                Some(expected_front_to_back.clone())
            );
            assert_eq!(
                window_grid_members_in_native_z_order(&[(a, 1), (b, 2)], &grids),
                expected_front_to_back.into_iter().rev().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn t_shaped_grid_obeys_all_vertical_and_horizontal_z_constraints() {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let [a, b, c, d] = [WindowGridMember::FloatingComposition(ids.insert(())), WindowGridMember::FloatingComposition(ids.insert(())), WindowGridMember::FloatingComposition(ids.insert(())), WindowGridMember::FloatingComposition(ids.insert(()))];
        let mut grids = WindowGridRegistry::default();

        assert!(grids.link_to_side(a, b, WindowGridDirection::Above));
        assert!(grids.link_to_side(a, c, WindowGridDirection::Right));
        assert!(grids.link_to_side(c, d, WindowGridDirection::Below));

        let members = grids.members_for(a);
        let links = grids.links_for(a).expect("the T-shaped grid has links");
        assert_eq!(
            floating_grid_members_in_native_front_to_back(&members, links),
            Some(vec![b, a, c, d])
        );
        for member in [a, b, c, d] {
            let selected_members = grids.members_for(member);
            let selected_links = grids
                .links_for(member)
                .expect("every T-shaped member resolves to the same grid");
            assert_eq!(
                floating_grid_members_in_native_front_to_back(&selected_members, selected_links),
                Some(vec![b, a, c, d]),
                "the move snapshot order must not depend on which member starts the drag"
            );
        }
        assert_eq!(
            window_grid_members_in_native_z_order(&[(a, 1), (b, 2), (c, 3), (d, 4)], &grids),
            vec![d, c, a, b]
        );
    }

    #[test]
    fn grid_members_are_kept_adjacent_in_native_z_order() {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let [a, b, c, d, e] = [
            WindowGridMember::FloatingComposition(ids.insert(())),
            WindowGridMember::FloatingComposition(ids.insert(())),
            WindowGridMember::FloatingComposition(ids.insert(())),
            WindowGridMember::FloatingComposition(ids.insert(())),
            WindowGridMember::FloatingComposition(ids.insert(())),
        ];
        let groups = [(a, 1), (c, 4), (d, 5), (e, 7), (b, 8)];
        let mut grids = WindowGridRegistry::default();
        assert!(grids.link_after(a, b));
        assert!(grids.link_after(c, d));

        assert_eq!(
            window_grid_members_in_native_z_order(&groups, &grids),
            vec![d, c, e, b, a],
            "grid members are raised back-to-front internally and ranked by their frontmost member"
        );
    }

    #[test]
    fn an_unlinked_floating_group_remains_a_single_z_order_block() {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let [a, b, c, d] = [WindowGridMember::FloatingComposition(ids.insert(())), WindowGridMember::FloatingComposition(ids.insert(())), WindowGridMember::FloatingComposition(ids.insert(())), WindowGridMember::FloatingComposition(ids.insert(()))];
        let groups = [(a, 3), (b, 1), (c, 2), (d, 4)];

        assert_eq!(
            window_grid_members_in_native_z_order(&groups, &WindowGridRegistry::default()),
            vec![b, c, a, d]
        );
    }
}

#[cfg(test)]
mod moving_window_width_tests {
    use super::{match_moving_window_width_to_target, WindowGridMember};
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [WindowGridMember; 2]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [
            WindowGridMember::FloatingComposition(ids.insert(())),
            WindowGridMember::FloatingComposition(ids.insert(())),
        ];
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
            120.0,
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
            120.0,
        )
        .expect("both windows must exist");

        assert_eq!(result[0].1, target_bounds);
        assert_eq!(result[1].1.size.width, 360.0);
        assert_eq!(result[1].1.size.height, moving_bounds.size.height);
    }
}

