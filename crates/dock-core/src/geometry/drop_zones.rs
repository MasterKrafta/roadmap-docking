use crate::command::{ColumnRef, DropTarget, DropZone};
use crate::geometry::{DockMetrics, DropZoneRect, LayoutRects, Point, Rect, Size, SplitterKind};
use crate::ids::{NodeId, PanelId};
use crate::model::{DockSide, Layout, Node};

use super::DropPayload;

// A wider hover band makes horizontal splitting easier to target than the resize handle alone.
// The preview remains anchored to the actual splitter rectangle.
const HORIZONTAL_DROP_ZONE_MIN_HEIGHT: f32 = 24.0;
/// Neutral pointer space separating split and center targets.
const DROP_ZONE_CENTER_GAP: f32 = 8.0;
/// Horizontal hit width around an insertion marker between tab headers.
const TAB_INSERTION_HIT_WIDTH: f32 = 12.0;

/// Build data-only rectangles for the docking overlay.
///
/// The vector is ordered by hit-test priority: floating groups (front to back),
/// tab insertion slots, dock edges, column edges, column ends, group splitters,
/// group centers, and finally the free Canvas area. The overlay can paint these
/// rectangles without depending on GPUI or other rendering code.
///
/// A `FreeArea` zone uses its `DropZoneRect.bounds` as the hit area. The actual
/// floating bounds are placed around the cursor by `resolve_drop`.
pub fn compute_drop_zones(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    metrics: &DockMetrics,
) -> Vec<DropZoneRect> {
    if !valid_payload(layout, payload) {
        return Vec::new();
    }

    let mut zones = Vec::new();
    let viewport = viewport_bounds(rects);
    if !valid_rect(viewport) {
        return zones;
    }

    // Floating windows are opaque to the drop overlay. Visit them front-to-back,
    // expose only the center hit area, and remember even invalid drop targets as
    // occluders so drops cannot pass through a dragged window onto content beneath it.
    let mut floating_occluders = Vec::new();
    for floating_rect in rects.floating_groups.iter().rev() {
        let bounds = floating_rect.bounds;
        if let Some(floating) = layout.floating_groups().get(floating_rect.floating) {
            let target = DropTarget::FloatingGroup {
                floating: floating_rect.floating,
                zone: DropZone::Center,
            };
            if !floating.panels.is_empty()
                && valid_rect(bounds)
                && valid_target(layout, payload, &target, metrics)
            {
                if let Some(center_bounds) =
                    floating_target_bounds(rects, floating_rect.floating, bounds, metrics)
                {
                    for visible_bounds in subtract_occlusion(center_bounds, &floating_occluders) {
                        push_zone(&mut zones, target.clone(), visible_bounds);
                    }
                }
            }
        }
        if valid_rect(bounds) {
            floating_occluders.push(bounds);
        }
    }
    // Only the front-to-back floating center zones are in the list so far.
    let floating_zone_count = zones.len();

    // Suppress lateral targets when adding the narrowest valid column would leave
    // the live Canvas narrower than its required document minimum.
    let can_add_column = can_add_column_in_viewport(layout, rects, payload, metrics);
    // Tab insertion targets have priority over center/edge targets only in the
    // narrow active band at the bottom of a tab strip; split/column neutral gaps stay empty.
    append_tab_insertion_zones(layout, rects, payload, &mut zones, metrics);

    let outer_zone_width =
        finite_positive(metrics.edge_drop_zone_width).min(viewport.size.width * 0.5);
    if can_add_column && outer_zone_width > 0.0 {
        push_zone(
            &mut zones,
            DropTarget::DockEdge {
                side: DockSide::Left,
            },
            Rect::new(
                Point::new(viewport.origin.x, viewport.origin.y),
                Size::new(outer_zone_width, viewport.size.height),
            ),
        );
        push_zone(
            &mut zones,
            DropTarget::DockEdge {
                side: DockSide::Right,
            },
            Rect::new(
                Point::new(
                    viewport.origin.x + viewport.size.width - outer_zone_width,
                    viewport.origin.y,
                ),
                Size::new(outer_zone_width, viewport.size.height),
            ),
        );
    }

    let edge_width = finite_positive(metrics.drop_edge_zone).min(viewport.size.width);
    if can_add_column && edge_width > 0.0 {
        append_column_edge_zones(rects, &mut zones, viewport, edge_width);
    }

    // The first and last horizontal zones belong to the entire column, not an
    // individual group's tabs or content.
    append_column_end_zones(layout, rects, payload, &mut zones, metrics);
    // Internal horizontal drops are only accepted on the actual splitter hit areas.
    append_group_splitter_zones(layout, rects, payload, &mut zones, metrics);
    append_group_zones(layout, rects, payload, &mut zones, metrics);

    // Canvas is protected as a docking destination, but its unoccupied surface is a
    // valid place to create a floating group only when the payload's minimum window
    // size can fit inside the current viewport.
    if is_docked_payload(layout, payload) && valid_rect(rects.canvas) {
        let canvas_center = Point::new(
            rects.canvas.origin.x + rects.canvas.size.width * 0.5,
            rects.canvas.origin.y + rects.canvas.size.height * 0.5,
        );
        if floating_bounds_at_point(layout, rects, payload, canvas_center, metrics).is_some() {
            push_zone(
                &mut zones,
                DropTarget::FreeArea {
                    bounds: Rect::default(),
                },
                rects.canvas,
            );
        }
    }

    // Hide every dock/free-area overlay rectangle underneath every floating window.
    // Keep the front-to-back floating targets already emitted above; their centers
    // were clipped against windows in front of them as they were added.
    let occluded_zones: Vec<_> = zones.drain(floating_zone_count..).collect();
    let mut visible_zones = zones;
    for zone in occluded_zones {
        for visible_bounds in subtract_occlusion(zone.bounds, &floating_occluders) {
            push_zone(&mut visible_zones, zone.target.clone(), visible_bounds);
        }
    }

    visible_zones
}

/// Resolve a drop point to a valid semantic target.
///
/// Geometry and metrics are explicit parameters because `Layout` is independent of a
/// window size and does not store logical rectangles. The same zone builder is used by
/// the overlay and the resolver, so painted and interactive zones stay consistent.
pub fn resolve_drop(
    layout: &Layout,
    point: Point,
    payload: DropPayload,
    rects: &LayoutRects,
    metrics: &DockMetrics,
) -> Option<DropTarget> {
    if !point.x.is_finite() || !point.y.is_finite() || !valid_payload(layout, payload) {
        return None;
    }

    // Do not click/drop through the frontmost floating window if its own target
    // is invalid for this payload (for example, dropping a group onto itself).
    for floating_rect in rects.floating_groups.iter().rev() {
        if !contains(floating_rect.bounds, point)
            || !layout
                .floating_groups()
                .contains_key(floating_rect.floating)
        {
            continue;
        }

        // The whole floating window blocks drop-through, but only its center area
        // is a docking target. Title bars and resize borders stay available to window
        // interactions and must not accidentally merge tabs.
        let target = DropTarget::FloatingGroup {
            floating: floating_rect.floating,
            zone: DropZone::Center,
        };
        if !valid_target(layout, payload, &target, metrics) {
            return None;
        }
        let center_bounds =
            floating_target_bounds(rects, floating_rect.floating, floating_rect.bounds, metrics)?;
        return contains(center_bounds, point).then_some(target);
    }

    let zones = compute_drop_zones(layout, rects, payload, metrics);
    let zone = zones.iter().find(|zone| contains(zone.bounds, point))?;
    match &zone.target {
        DropTarget::FreeArea { .. } => {
            let bounds = floating_bounds_at_point(layout, rects, payload, point, metrics)?;
            Some(DropTarget::FreeArea { bounds })
        }
        DropTarget::TabInsertion { group, index } => {
            let target = DropTarget::TabInsertion {
                group: *group,
                index: *index,
            };
            valid_target(layout, payload, &target, metrics).then_some(target)
        }
        target if valid_target(layout, payload, target, metrics) => Some((*target).clone()),
        _ => None,
    }
}

fn append_tab_insertion_zones(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    zones: &mut Vec<DropZoneRect>,
    metrics: &DockMetrics,
) {
    let DropPayload::Panel(panel) = payload else {
        return;
    };
    let Some(source_group) = find_dock_group_for_panel(layout, panel) else {
        return;
    };
    let source_index = match layout.nodes().get(source_group) {
        Some(Node::Tabs(tabs)) => tabs.panels.iter().position(|candidate| *candidate == panel),
        _ => None,
    };

    for strip in &rects.tab_strips {
        // TabInsertion is a reorder gesture, not a way to move a panel into another group.
        // Cross-group moves use that group's Center target instead, so the column's flex
        // spacer and neighbouring tab strips never become insertion destinations.
        if strip.group != source_group
            || !is_docked_group(layout, strip.group)
            || !valid_rect(strip.bounds)
        {
            continue;
        }
        let Some(Node::Tabs(tabs)) = layout.nodes().get(strip.group) else {
            continue;
        };
        if tabs.panels.is_empty() {
            continue;
        }

        let mut headers: Vec<_> = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == strip.group)
            .collect();
        headers.sort_by_key(|header| header.index);
        if headers.len() != tabs.panels.len() {
            continue;
        }

        // Track the actual tab strip vertically: users should be able to reorder by
        // dragging horizontally at any height, not only through the bottom quarter.
        // Horizontal hit targets stay narrow and only exist at boundaries in this group.
        let hit_top = strip.bounds.origin.y;
        let hit_height = strip.bounds.size.height.max(0.0);
        if hit_height <= 0.0 {
            continue;
        }

        for index in 0..=headers.len() {
            if source_index.is_some_and(|source| index == source || index == source + 1) {
                // The two boundaries around the dragged tab would be a no-op.
                continue;
            }
            let target = DropTarget::TabInsertion {
                group: strip.group,
                index,
            };
            if !valid_target(layout, payload, &target, metrics) {
                continue;
            }

            let (x, adjacent_width) = if index < headers.len() {
                (
                    headers[index].bounds.origin.x,
                    headers[index].bounds.size.width,
                )
            } else {
                let last = headers.last().expect("non-empty headers checked above");
                (
                    last.bounds.origin.x + last.bounds.size.width,
                    last.bounds.size.width,
                )
            };
            let hit_width = TAB_INSERTION_HIT_WIDTH.min(adjacent_width.max(0.0));
            let bounds = Rect::new(
                Point::new(x - hit_width * 0.5, hit_top),
                Size::new(hit_width, hit_height),
            );
            push_zone(zones, target, bounds);
        }
    }
}

fn append_column_edge_zones(
    rects: &LayoutRects,
    zones: &mut Vec<DropZoneRect>,
    viewport: Rect,
    zone_width: f32,
) {
    for side in [DockSide::Left, DockSide::Right] {
        let mut columns: Vec<_> = rects
            .columns
            .iter()
            .filter(|column| column.side == side && valid_rect(column.bounds))
            .collect();
        columns.sort_by_key(|column| column.index);
        if columns.is_empty() {
            continue;
        }

        // Each internal boundary is represented once: a new column to the right of
        // the column on its left. That avoids two coincident zones at the same x.
        for pair in columns.windows(2) {
            let left_column = pair[0];
            let boundary_x = left_column.bounds.origin.x + left_column.bounds.size.width;
            let left = (boundary_x - zone_width * 0.5).max(viewport.origin.x);
            let right =
                (boundary_x + zone_width * 0.5).min(viewport.origin.x + viewport.size.width);
            push_zone(
                zones,
                DropTarget::Column {
                    column: ColumnRef::new(side, left_column.index),
                    zone: DropZone::Right,
                },
                Rect::new(
                    Point::new(left, viewport.origin.y),
                    Size::new((right - left).max(0.0), viewport.size.height),
                ),
            );
        }

        match side {
            DockSide::Left => {
                // The screen-facing edge of the left dock is a DockEdge target.
                // Its other edge faces the Canvas and creates a neighboring column.
                if let Some(column) = columns.last() {
                    let x = column.bounds.origin.x + column.bounds.size.width;
                    push_zone(
                        zones,
                        DropTarget::Column {
                            column: ColumnRef::new(side, column.index),
                            zone: DropZone::Right,
                        },
                        vertical_strip(x, zone_width, viewport),
                    );
                }
            }
            DockSide::Right => {
                // The screen-facing edge of the right dock is a DockEdge target.
                // Its other edge faces the Canvas and creates a neighboring column.
                if let Some(column) = columns.first() {
                    let x = column.bounds.origin.x;
                    push_zone(
                        zones,
                        DropTarget::Column {
                            column: ColumnRef::new(side, column.index),
                            zone: DropZone::Left,
                        },
                        vertical_strip(x, zone_width, viewport),
                    );
                }
            }
        }
    }
}

fn append_column_end_zones(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    zones: &mut Vec<DropZoneRect>,
    metrics: &DockMetrics,
) {
    let requested_height = finite_positive(metrics.splitter_hit_zone);
    if requested_height <= 0.0 {
        return;
    }
    let requested_height = requested_height.max(HORIZONTAL_DROP_ZONE_MIN_HEIGHT);

    for column in &rects.columns {
        if !valid_rect(column.bounds) {
            continue;
        }
        let bounds = column.bounds;
        let column_bottom = bounds.origin.y + bounds.size.height;
        let height = requested_height.min(bounds.size.height * 0.5);
        if height <= 0.0 {
            continue;
        }
        let header_bottom = rects
            .column_headers
            .iter()
            .find(|header| header.side == column.side && header.index == column.index)
            .filter(|header| valid_rect(header.bounds))
            .map(|header| header.bounds.origin.y + header.bounds.size.height)
            .unwrap_or(bounds.origin.y)
            .max(bounds.origin.y)
            .min(column_bottom);
        let column_ref = ColumnRef::new(column.side, column.index);
        let top_target = DropTarget::Column {
            column: column_ref,
            zone: DropZone::Top,
        };
        if valid_target(layout, payload, &top_target, metrics)
            && can_add_group_to_column(
                layout,
                rects,
                payload,
                column.side,
                column.index,
                metrics,
            )
        {
            let top = (header_bottom - height * 0.5).max(bounds.origin.y);
            let bottom = (header_bottom + height * 0.5).min(column_bottom);
            push_zone(
                zones,
                top_target,
                Rect::new(
                    Point::new(bounds.origin.x, top),
                    Size::new(bounds.size.width, (bottom - top).max(0.0)),
                ),
            );
        }

        let bottom_target = DropTarget::Column {
            column: column_ref,
            zone: DropZone::Bottom,
        };
        if valid_target(layout, payload, &bottom_target, metrics)
            && can_add_group_to_column(
                layout,
                rects,
                payload,
                column.side,
                column.index,
                metrics,
            )
        {
            push_zone(
                zones,
                bottom_target,
                Rect::new(
                    Point::new(
                        bounds.origin.x,
                        (column_bottom - height).max(bounds.origin.y),
                    ),
                    Size::new(bounds.size.width, height),
                ),
            );
        }
    }
}

fn append_group_splitter_zones(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    zones: &mut Vec<DropZoneRect>,
    metrics: &DockMetrics,
) {
    for column in &rects.columns {
        let mut groups: Vec<_> = rects
            .groups
            .iter()
            .filter(|group| {
                group.bounds.origin.x == column.bounds.origin.x
                    && group.bounds.size.width == column.bounds.size.width
                    && valid_rect(group.bounds)
            })
            .collect();
        groups.sort_by(|left, right| left.bounds.origin.y.total_cmp(&right.bounds.origin.y));

        for pair in groups.windows(2) {
            let previous = pair[0];
            let next = pair[1];
            let gap_top = previous.bounds.origin.y + previous.bounds.size.height;
            let gap_bottom = next.bounds.origin.y;
            if gap_bottom <= gap_top {
                continue;
            }

            // Anchor the split target on the actual separator, but make the hover
            // band wider than the resize handle. Clip it to the adjacent group
            // centers so it cannot consume either group's whole center target or
            // overlap a neighboring split target.
            let Some(splitter) = rects.splitters.iter().find(|splitter| {
                splitter.kind == SplitterKind::Group
                    && splitter.bounds.origin.x == column.bounds.origin.x
                    && splitter.bounds.size.width == column.bounds.size.width
                    && {
                        let center_y =
                            splitter.bounds.origin.y + splitter.bounds.size.height * 0.5;
                        center_y >= gap_top && center_y <= gap_bottom
                    }
            }) else {
                continue;
            };

            let drop_bounds = group_split_drop_bounds(
                splitter.bounds,
                previous.bounds,
                next.bounds,
            );
            // Both directions are valid at the same divider: the upper half inserts
            // after the preceding group (Bottom), and the lower half inserts before
            // the following group (Top). Keeping the hit rectangles disjoint gives
            // the pointer one deterministic semantic target without losing the split band.
            let split_y = drop_bounds.origin.y + drop_bounds.size.height * 0.5;
            let upper_bounds = Rect::new(
                drop_bounds.origin,
                Size::new(drop_bounds.size.width, (split_y - drop_bounds.origin.y).max(0.0)),
            );
            let lower_bounds = Rect::new(
                Point::new(drop_bounds.origin.x, split_y),
                Size::new(
                    drop_bounds.size.width,
                    (drop_bounds.origin.y + drop_bounds.size.height - split_y).max(0.0),
                ),
            );
            let column_can_split = can_add_group_to_column(
                layout,
                rects,
                payload,
                column.side,
                column.index,
                metrics,
            );

            let bottom_target = DropTarget::Group {
                group: previous.group,
                zone: DropZone::Bottom,
            };
            if upper_bounds.size.height > 0.0
                && column_can_split
                && valid_target(layout, payload, &bottom_target, metrics)
                && can_split_group_vertically(layout, rects, payload, previous.group, metrics)
            {
                push_zone(zones, bottom_target, upper_bounds);
            }

            let top_target = DropTarget::Group {
                group: next.group,
                zone: DropZone::Top,
            };
            if lower_bounds.size.height > 0.0
                && column_can_split
                && valid_target(layout, payload, &top_target, metrics)
                && can_split_group_vertically(layout, rects, payload, next.group, metrics)
            {
                push_zone(zones, top_target, lower_bounds);
            }
        }
    }
}

/// Expand a group-split hit area around the actual splitter, limited by the
/// vertical centers of the two groups so the target cannot swallow either center.
fn group_split_drop_bounds(splitter: Rect, previous: Rect, next: Rect) -> Rect {
    let splitter_center_y = splitter.origin.y + splitter.size.height * 0.5;
    let desired_height = HORIZONTAL_DROP_ZONE_MIN_HEIGHT.max(splitter.size.height);
    let top_limit = previous.origin.y + previous.size.height * 0.5;
    let bottom_limit = next.origin.y + next.size.height * 0.5;
    let top = (splitter_center_y - desired_height * 0.5).max(top_limit);
    let bottom = (splitter_center_y + desired_height * 0.5).min(bottom_limit);
    Rect::new(
        Point::new(splitter.origin.x, top),
        Size::new(splitter.size.width, (bottom - top).max(0.0)),
    )
}

fn append_group_zones(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    zones: &mut Vec<DropZoneRect>,
    metrics: &DockMetrics,
) {
    let edge = finite_positive(metrics.drop_edge_zone);
    if edge <= 0.0 {
        return;
    }

    for group_rect in &rects.groups {
        if !is_docked_group(layout, group_rect.group) || !valid_rect(group_rect.bounds) {
            continue;
        }
        let bounds = group_rect.bounds;
        let horizontal_edge = edge.min(bounds.size.width * 0.5);
        let group_bottom = bounds.origin.y + bounds.size.height;
        let group_center_y = bounds.origin.y + bounds.size.height * 0.5;
        let mut center_top = bounds.origin.y;
        let mut center_bottom = group_bottom;

        // The first and last group also need a neutral gap between their Center
        // target and the column-level Top / Bottom targets.
        if let Some(column) = rects.columns.iter().find(|column| {
            column.bounds.origin.x == bounds.origin.x
                && column.bounds.size.width == bounds.size.width
        }) {
            let column_bounds = column.bounds;
            let column_bottom = column_bounds.origin.y + column_bounds.size.height;
            let column_ref = ColumnRef::new(column.side, column.index);
            let height = finite_positive(metrics.splitter_hit_zone)
                .max(HORIZONTAL_DROP_ZONE_MIN_HEIGHT)
                .min(column_bounds.size.height * 0.5);
            let header_bottom = rects
                .column_headers
                .iter()
                .find(|header| header.side == column.side && header.index == column.index)
                .filter(|header| valid_rect(header.bounds))
                .map(|header| header.bounds.origin.y + header.bounds.size.height)
                .unwrap_or(column_bounds.origin.y)
                .max(column_bounds.origin.y)
                .min(column_bottom);

            if (bounds.origin.y - header_bottom).abs() < 0.001 {
                let target = DropTarget::Column {
                    column: column_ref,
                    zone: DropZone::Top,
                };
                if valid_target(layout, payload, &target, metrics) {
                    let top_zone_bottom = (header_bottom + height * 0.5).min(column_bottom);
                    center_top = center_top.max(top_zone_bottom + DROP_ZONE_CENTER_GAP);
                }
            }

            if (group_bottom - column_bottom).abs() < 0.001 {
                let target = DropTarget::Column {
                    column: column_ref,
                    zone: DropZone::Bottom,
                };
                if valid_target(layout, payload, &target, metrics) {
                    let bottom_zone_top = (column_bottom - height).max(column_bounds.origin.y);
                    center_bottom =
                        center_bottom.min(bottom_zone_top - DROP_ZONE_CENTER_GAP);
                }
            }
        }

        // Keep a neutral vertical gap between the wider internal split target and
        // the adjacent group's Center target.
        for splitter in rects.splitters.iter().filter(|splitter| {
            splitter.kind == SplitterKind::Group
                && splitter.bounds.origin.x == bounds.origin.x
                && splitter.bounds.size.width == bounds.size.width
        }) {
            let splitter_center_y =
                splitter.bounds.origin.y + splitter.bounds.size.height * 0.5;
            let split_half_height =
                HORIZONTAL_DROP_ZONE_MIN_HEIGHT.max(splitter.bounds.size.height) * 0.5;

            if splitter_center_y <= bounds.origin.y
                && splitter.bounds.origin.y + splitter.bounds.size.height >= bounds.origin.y
            {
                // A splitter immediately above this group: leave the gap below its hit zone.
                center_top = center_top.max(
                    (splitter_center_y + split_half_height).min(group_center_y)
                        + DROP_ZONE_CENTER_GAP,
                );
            } else if splitter_center_y >= group_bottom
                && splitter.bounds.origin.y <= group_bottom
            {
                // A splitter immediately below this group: leave the gap above its hit zone.
                center_bottom = center_bottom.min(
                    (splitter_center_y - split_half_height).max(group_center_y)
                        - DROP_ZONE_CENTER_GAP,
                );
            }
        }

        if center_bottom <= center_top {
            continue;
        }

        let center_bounds = Rect::new(
            Point::new(bounds.origin.x + horizontal_edge, center_top),
            Size::new(
                (bounds.size.width - horizontal_edge * 2.0).max(0.0),
                (center_bottom - center_top).max(0.0),
            ),
        );
        let target = DropTarget::Group {
            group: group_rect.group,
            zone: DropZone::Center,
        };
        if valid_target(layout, payload, &target, metrics) {
            push_zone(zones, target, center_bounds);
        }
    }
}

fn valid_payload(layout: &Layout, payload: DropPayload) -> bool {
    match payload {
        DropPayload::Panel(panel) => {
            layout.panels().contains_key(panel)
                && (find_dock_group_for_panel(layout, panel).is_some()
                    || layout
                        .floating_groups()
                        .values()
                        .any(|floating| floating.panels.contains(&panel)))
        }
        DropPayload::Group(group) => {
            is_docked_group(layout, group)
                && matches!(layout.nodes().get(group), Some(Node::Tabs(tabs)) if !tabs.panels.is_empty())
        }
        DropPayload::FloatingGroup(floating) => layout
            .floating_groups()
            .get(floating)
            .is_some_and(|group| !group.panels.is_empty()),
    }
}

fn valid_target(
    layout: &Layout,
    payload: DropPayload,
    target: &DropTarget,
    metrics: &DockMetrics,
) -> bool {
    match target {
        DropTarget::Group { group, zone } => {
            let can_target_group = match zone {
                // A single panel may originate in either a dock group or a floating group.
                // Other split operations remain limited to docked payloads because they
                // create dock topology rather than merely joining an existing tab group.
                DropZone::Center => {
                    can_target_dock(layout, payload)
                        || matches!(payload, DropPayload::Panel(panel)
                            if is_floating_panel(layout, panel))
                }
                DropZone::Top | DropZone::Bottom
                    if layout.profile() == crate::model::Profile::Photoshop =>
                {
                    can_target_dock(layout, payload)
                }
                DropZone::Top | DropZone::Bottom | DropZone::Left | DropZone::Right => false,
            };
            if !can_target_group
                || !is_docked_group(layout, *group)
                || !matches!(layout.nodes().get(*group), Some(Node::Tabs(_)))
                || matches!(payload, DropPayload::Group(source) if source == *group)
            {
                return false;
            }
            if *zone == DropZone::Center {
                if let DropPayload::Panel(panel) = payload {
                    if matches!(layout.nodes().get(*group), Some(Node::Tabs(tabs)) if tabs.panels.contains(&panel))
                    {
                        return false;
                    }
                }
            }
            true
        }
        DropTarget::TabInsertion { group, index } => {
            if !can_target_dock(layout, payload) {
                return false;
            }
            let Some(Node::Tabs(tabs)) = layout.nodes().get(*group) else {
                return false;
            };
            if !is_docked_group(layout, *group) || *index > tabs.panels.len() {
                return false;
            }

            match payload {
                DropPayload::Panel(panel) => {
                    find_dock_group_for_panel(layout, panel) == Some(*group)
                }
                DropPayload::Group(source) => {
                    source != *group && *index > 0 && *index < tabs.panels.len()
                }
                DropPayload::FloatingGroup(source) => {
                    *index > 0
                        && *index < tabs.panels.len()
                        && layout
                            .floating_groups()
                            .get(source)
                            .is_some_and(|floating| !floating.panels.is_empty())
                }
            }
        }
        DropTarget::Column { column, zone } => {
            if layout.profile() != crate::model::Profile::Photoshop
                || !can_target_dock(layout, payload)
                || !column_exists(layout, *column)
            {
                return false;
            }
            match zone {
                DropZone::Left | DropZone::Right => can_add_column(layout, metrics),
                DropZone::Top | DropZone::Bottom => true,
                DropZone::Center => false,
            }
        }
        DropTarget::DockEdge { .. } => {
            can_target_dock(layout, payload) && can_add_column(layout, metrics)
        },
        DropTarget::FloatingGroup { floating, zone } => {
            *zone == DropZone::Center
                && layout.floating_groups().get(*floating).is_some_and(|target| {
                    !target.panels.is_empty()
                        && !matches!(payload, DropPayload::FloatingGroup(source) if source == *floating)
                        && !matches!(payload, DropPayload::Panel(panel) if target.panels.contains(&panel))
                })
        }
        DropTarget::FreeArea { .. } => is_docked_payload(layout, payload),
    }
}

fn is_docked_payload(layout: &Layout, payload: DropPayload) -> bool {
    match payload {
        DropPayload::Panel(panel) => find_dock_group_for_panel(layout, panel).is_some(),
        DropPayload::Group(group) => is_docked_group(layout, group),
        DropPayload::FloatingGroup(_) => false,
    }
}

fn is_floating_panel(layout: &Layout, panel: PanelId) -> bool {
    layout
        .floating_groups()
        .values()
        .any(|floating| floating.panels.contains(&panel))
}

fn can_target_dock(layout: &Layout, payload: DropPayload) -> bool {
    match payload {
        DropPayload::Panel(panel) => find_dock_group_for_panel(layout, panel).is_some(),
        DropPayload::Group(group) => is_docked_group(layout, group),
        DropPayload::FloatingGroup(floating) => layout
            .floating_groups()
            .get(floating)
            .is_some_and(|group| !group.panels.is_empty()),
    }
}

fn find_dock_group_for_panel(layout: &Layout, panel: PanelId) -> Option<NodeId> {
    layout
        .dock_areas()
        .iter()
        .flat_map(|area| area.columns.iter())
        .flat_map(|column| column.groups.iter().copied())
        .find(|group| {
            matches!(
                layout.nodes().get(*group),
                Some(Node::Tabs(tabs)) if tabs.panels.contains(&panel)
            )
        })
}

fn is_docked_group(layout: &Layout, group: NodeId) -> bool {
    layout.dock_areas().iter().any(|area| {
        area.columns
            .iter()
            .any(|column| column.groups.contains(&group))
    })
}

fn column_exists(layout: &Layout, column: ColumnRef) -> bool {
    layout
        .dock_areas()
        .iter()
        .find(|area| area.side == column.side)
        .is_some_and(|area| column.index < area.columns.len())
}

fn can_add_column(layout: &Layout, metrics: &DockMetrics) -> bool {
    layout.profile() == crate::model::Profile::Photoshop
        && column_count(layout) < metrics.max_columns
}

/// Includes live viewport feasibility in addition to the model's column-count limit.
fn can_add_column_in_viewport(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    metrics: &DockMetrics,
) -> bool {
    if !can_add_column(layout, metrics) {
        return false;
    }
    let Some(width) = minimum_docked_column_width(layout, payload, metrics) else {
        return false;
    };
    let canvas_width = rects.canvas.size.width;
    canvas_width.is_finite()
        && canvas_width >= width
        && canvas_width - width + 1.0e-3 >= metrics.min_document_width
}

/// Width needed if this payload becomes a new expanded Photoshop column.
fn minimum_docked_column_width(
    layout: &Layout,
    payload: DropPayload,
    metrics: &DockMetrics,
) -> Option<f32> {
    let panels = payload_panels(layout, payload)?;
    let mut width = metrics.default_column_width.max(metrics.min_column_width);
    for panel in panels {
        let panel = layout.panels().get(panel)?;
        let minimum = panel.constraints().min_size().width;
        if !minimum.is_finite() || minimum < 0.0 {
            return None;
        }
        width = width.max(minimum);
    }
    (width.is_finite() && width > 0.0).then_some(width)
}

fn payload_panels(layout: &Layout, payload: DropPayload) -> Option<Vec<PanelId>> {
    match payload {
        DropPayload::Panel(panel) => layout.panels().contains_key(panel).then_some(vec![panel]),
        DropPayload::Group(group) => match layout.nodes().get(group)? {
            Node::Tabs(tabs) if !tabs.panels.is_empty() => Some(tabs.panels.clone()),
            _ => None,
        },
        DropPayload::FloatingGroup(floating) => {
            let group = layout.floating_groups().get(floating)?;
            (!group.panels.is_empty()).then_some(group.panels.clone())
        }
    }
}

fn minimum_group_height_for_panels(
    layout: &Layout,
    panels: &[PanelId],
    metrics: &DockMetrics,
) -> Option<f32> {
    if panels.is_empty() {
        return Some(0.0);
    }
    let mut height = metrics.min_group_height;
    for panel_id in panels {
        let panel = layout.panels().get(*panel_id)?;
        let minimum = panel.constraints().min_size().height;
        if !minimum.is_finite() || minimum < 0.0 {
            return None;
        }
        height = height.max(minimum);
    }
    (height.is_finite() && height >= 0.0).then_some(height)
}

fn minimum_payload_group_height(
    layout: &Layout,
    payload: DropPayload,
    metrics: &DockMetrics,
) -> Option<f32> {
    minimum_group_height_for_panels(layout, &payload_panels(layout, payload)?, metrics)
}

/// Minimum remaining height of a dock group after removing the dragged payload from it.
fn minimum_group_height_after_drop(
    layout: &Layout,
    group: NodeId,
    payload: DropPayload,
    metrics: &DockMetrics,
) -> Option<f32> {
    let Some(Node::Tabs(tabs)) = layout.nodes().get(group) else {
        return None;
    };
    let mut panels = tabs.panels.clone();
    match payload {
        DropPayload::Panel(panel) if find_dock_group_for_panel(layout, panel) == Some(group) => {
            panels.retain(|candidate| *candidate != panel);
        }
        DropPayload::Group(source) if source == group => panels.clear(),
        _ => {}
    }
    minimum_group_height_for_panels(layout, &panels, metrics)
}

/// Whether a side drop can allocate a new group without violating column minimum heights.
fn can_add_group_to_column(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    side: DockSide,
    index: usize,
    metrics: &DockMetrics,
) -> bool {
    let Some(column) = rects
        .columns
        .iter()
        .find(|column| column.side == side && column.index == index && valid_rect(column.bounds))
    else {
        return false;
    };
    let Some(model_column) = layout
        .dock_areas()
        .iter()
        .find(|area| area.side == side)
        .and_then(|area| area.columns.get(index))
    else {
        return false;
    };

    let mut required_heights = Vec::with_capacity(model_column.groups.len() + 1);
    for group in &model_column.groups {
        let Some(height) = minimum_group_height_after_drop(layout, *group, payload, metrics)
        else {
            return false;
        };
        if height > 0.0 {
            required_heights.push(height);
        }
    }
    let Some(incoming_height) = minimum_payload_group_height(layout, payload, metrics) else {
        return false;
    };
    required_heights.push(incoming_height);
    let group_count = required_heights.len();
    let required_height: f32 = required_heights.iter().sum::<f32>()
        + metrics.splitter_thickness.max(0.0) * group_count.saturating_sub(1) as f32;
    required_height.is_finite()
        && required_height <= column.bounds.size.height + 1.0e-3
}

/// The target group's current allocation must fit both sides of the new split.
fn can_split_group_vertically(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    group: NodeId,
    metrics: &DockMetrics,
) -> bool {
    let Some(bounds) = rects.groups.iter().find(|rect| rect.group == group) else {
        return false;
    };
    let Some(existing_height) = minimum_group_height_after_drop(layout, group, payload, metrics)
    else {
        return false;
    };
    let Some(incoming_height) = minimum_payload_group_height(layout, payload, metrics) else {
        return false;
    };
    let required = existing_height + incoming_height + metrics.splitter_thickness.max(0.0);
    required.is_finite() && required <= bounds.bounds.size.height + 1.0e-3
}

fn column_count(layout: &Layout) -> usize {
    layout
        .dock_areas()
        .iter()
        .map(|area| area.columns.len())
        .sum()
}

fn floating_bounds_at_point(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    point: Point,
    metrics: &DockMetrics,
) -> Option<Rect> {
    let (desired_width, desired_height, minimum_width, minimum_height) =
        desired_floating_size(layout, rects, payload, metrics)?;
    let viewport = viewport_bounds(rects);
    if !valid_rect(viewport) {
        return None;
    }

    if viewport.size.width < minimum_width || viewport.size.height < minimum_height {
        return None;
    }
    let width = desired_width.min(viewport.size.width);
    let height = desired_height.min(viewport.size.height);
    if width < minimum_width || height < minimum_height {
        return None;
    }

    let max_x = viewport.origin.x + viewport.size.width - width;
    let max_y = viewport.origin.y + viewport.size.height - height;
    let x = (point.x - width * 0.5).clamp(viewport.origin.x, max_x);
    let y = (point.y - height * 0.5).clamp(viewport.origin.y, max_y);
    Some(Rect::new(Point::new(x, y), Size::new(width, height)))
}

/// Return the true center drop area of a floating window, excluding its title bar,
/// resize borders, and a small safety strip along the bottom and sides.
fn floating_target_bounds(
    rects: &LayoutRects,
    floating: crate::ids::FloatingGroupId,
    bounds: Rect,
    metrics: &DockMetrics,
) -> Option<Rect> {
    if !valid_rect(bounds) {
        return None;
    }

    let fallback_title_height =
        finite_positive(metrics.column_header_height).min(bounds.size.height);
    let title_bottom = rects
        .floating_titles
        .iter()
        .find(|title| title.floating == floating)
        .map(|title| title.bounds.origin.y + title.bounds.size.height)
        .filter(|bottom| bottom.is_finite())
        .unwrap_or(bounds.origin.y + fallback_title_height);

    let horizontal_inset = finite_positive(metrics.drop_edge_zone).min(bounds.size.width * 0.5);
    let vertical_inset = finite_positive(metrics.drop_edge_zone).min(bounds.size.height * 0.5);
    let left = bounds.origin.x + horizontal_inset;
    let right = bounds.origin.x + bounds.size.width - horizontal_inset;
    let top = title_bottom.max(bounds.origin.y + vertical_inset);
    let bottom = bounds.origin.y + bounds.size.height - vertical_inset;

    if !left.is_finite()
        || !right.is_finite()
        || !top.is_finite()
        || !bottom.is_finite()
        || right <= left
        || bottom <= top
    {
        return None;
    }

    Some(Rect::new(
        Point::new(left, top),
        Size::new(right - left, bottom - top),
    ))
}

/// Remove any area covered by floating windows. Each subtraction returns up to four
/// non-overlapping rectangles, so overlay geometry and resolve_drop agree on occlusion.
fn subtract_occlusion(source: Rect, occluders: &[Rect]) -> Vec<Rect> {
    if !valid_rect(source) {
        return Vec::new();
    }

    let mut visible = vec![source];
    for occluder in occluders {
        visible = visible
            .into_iter()
            .flat_map(|piece| subtract_rect(piece, *occluder))
            .collect();
        if visible.is_empty() {
            break;
        }
    }
    visible
}

fn subtract_rect(source: Rect, occluder: Rect) -> Vec<Rect> {
    if !valid_rect(source) {
        return Vec::new();
    }
    if !valid_rect(occluder) {
        return vec![source];
    }

    let source_left = source.origin.x;
    let source_top = source.origin.y;
    let source_right = source_left + source.size.width;
    let source_bottom = source_top + source.size.height;
    let occluder_left = occluder.origin.x;
    let occluder_top = occluder.origin.y;
    let occluder_right = occluder_left + occluder.size.width;
    let occluder_bottom = occluder_top + occluder.size.height;

    if !source_right.is_finite()
        || !source_bottom.is_finite()
        || !occluder_right.is_finite()
        || !occluder_bottom.is_finite()
    {
        return vec![source];
    }

    let intersection_left = source_left.max(occluder_left);
    let intersection_top = source_top.max(occluder_top);
    let intersection_right = source_right.min(occluder_right);
    let intersection_bottom = source_bottom.min(occluder_bottom);
    if intersection_right <= intersection_left || intersection_bottom <= intersection_top {
        return vec![source];
    }

    let mut visible = Vec::with_capacity(4);
    if intersection_top > source_top {
        visible.push(Rect::new(
            Point::new(source_left, source_top),
            Size::new(source.size.width, intersection_top - source_top),
        ));
    }
    if intersection_bottom < source_bottom {
        visible.push(Rect::new(
            Point::new(source_left, intersection_bottom),
            Size::new(source.size.width, source_bottom - intersection_bottom),
        ));
    }
    if intersection_left > source_left {
        visible.push(Rect::new(
            Point::new(source_left, intersection_top),
            Size::new(
                intersection_left - source_left,
                intersection_bottom - intersection_top,
            ),
        ));
    }
    if intersection_right < source_right {
        visible.push(Rect::new(
            Point::new(intersection_right, intersection_top),
            Size::new(
                source_right - intersection_right,
                intersection_bottom - intersection_top,
            ),
        ));
    }

    visible
        .into_iter()
        .filter(|rect| valid_rect(*rect))
        .collect()
}

fn desired_floating_size(
    layout: &Layout,
    rects: &LayoutRects,
    payload: DropPayload,
    metrics: &DockMetrics,
) -> Option<(f32, f32, f32, f32)> {
    let title_height = finite_positive(metrics.column_header_height);
    let tab_height = finite_positive(metrics.tab_strip_height);
    match payload {
        DropPayload::Panel(panel_id) => {
            let panel = layout.panels().get(panel_id)?;
            let constraints = panel.constraints();
            let minimum = constraints.min_size();
            let preferred = constraints.preferred_size();
            let minimum_content_height =
                finite_positive(metrics.min_group_height).max(finite_positive(minimum.height));
            let width = finite_positive(metrics.default_column_width)
                .max(finite_positive(metrics.min_column_width))
                .max(finite_positive(preferred.width))
                .max(finite_positive(minimum.width));
            let minimum_width =
                finite_positive(metrics.min_column_width).max(finite_positive(minimum.width));
            let height = finite_positive(preferred.height).max(minimum_content_height)
                + title_height
                + tab_height;
            let minimum_height = minimum_content_height + title_height + tab_height;
            Some((width, height, minimum_width, minimum_height))
        }
        DropPayload::Group(group_id) => {
            let Node::Tabs(tabs) = layout.nodes().get(group_id)? else {
                return None;
            };
            let bounds = rects
                .groups
                .iter()
                .find(|group| group.group == group_id)?
                .bounds;
            let mut minimum_width = finite_positive(metrics.min_column_width);
            let mut minimum_content_height = finite_positive(metrics.min_group_height);
            for panel_id in &tabs.panels {
                let constraints = layout.panels().get(*panel_id)?.constraints();
                minimum_width = minimum_width.max(finite_positive(constraints.min_size().width));
                minimum_content_height =
                    minimum_content_height.max(finite_positive(constraints.min_size().height));
            }
            let width = bounds
                .size
                .width
                .max(finite_positive(metrics.default_column_width))
                .max(minimum_width);
            let height = bounds.size.height + title_height;
            // A floating group keeps its own tab strip as well as the outer title bar.
            // Require room for both chrome rows plus the active panel's minimum content.
            Some((
                width,
                height,
                minimum_width,
                minimum_content_height + title_height + tab_height,
            ))
        }
        DropPayload::FloatingGroup(floating_id) => {
            let group = layout.floating_groups().get(floating_id)?;
            let bounds = group.bounds;
            let mut minimum_width = finite_positive(metrics.min_column_width);
            let mut minimum_height = finite_positive(metrics.min_group_height);
            for panel_id in &group.panels {
                let constraints = layout.panels().get(*panel_id)?.constraints();
                minimum_width = minimum_width.max(finite_positive(constraints.min_size().width));
                minimum_height = minimum_height.max(finite_positive(constraints.min_size().height));
            }
            Some((
                bounds.size.width,
                bounds.size.height,
                minimum_width,
                minimum_height,
            ))
        }
    }
}

fn viewport_bounds(rects: &LayoutRects) -> Rect {
    let mut left = rects.canvas.origin.x;
    let mut top = rects.canvas.origin.y;
    let mut right = rects.canvas.origin.x + rects.canvas.size.width;
    let mut bottom = rects.canvas.origin.y + rects.canvas.size.height;

    for area in &rects.dock_areas {
        left = left.min(area.bounds.origin.x);
        top = top.min(area.bounds.origin.y);
        right = right.max(area.bounds.origin.x + area.bounds.size.width);
        bottom = bottom.max(area.bounds.origin.y + area.bounds.size.height);
    }

    Rect::new(
        Point::new(left, top),
        Size::new((right - left).max(0.0), (bottom - top).max(0.0)),
    )
}

fn vertical_strip(x: f32, width: f32, viewport: Rect) -> Rect {
    let left = (x - width * 0.5).max(viewport.origin.x);
    let right = (x + width * 0.5).min(viewport.origin.x + viewport.size.width);
    Rect::new(
        Point::new(left, viewport.origin.y),
        Size::new((right - left).max(0.0), viewport.size.height),
    )
}

fn push_zone(zones: &mut Vec<DropZoneRect>, target: DropTarget, bounds: Rect) {
    if valid_rect(bounds) {
        zones.push(DropZoneRect { target, bounds });
    }
}

fn valid_rect(rect: Rect) -> bool {
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
        && rect.size.width > 0.0
        && rect.size.height > 0.0
}

fn finite_positive(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn contains(bounds: Rect, point: Point) -> bool {
    if !valid_rect(bounds) || !point.x.is_finite() || !point.y.is_finite() {
        return false;
    }
    point.x >= bounds.origin.x
        && point.x < bounds.origin.x + bounds.size.width
        && point.y >= bounds.origin.y
        && point.y < bounds.origin.y + bounds.size.height
}

#[cfg(test)]
mod tests {
    use super::{
        HORIZONTAL_DROP_ZONE_MIN_HEIGHT, compute_drop_zones, contains, resolve_drop,
    };
    use crate::{
        ColumnRef, DockCommand, DockMetrics, DockSide, DropPayload, DropTarget, DropZone,
        FloatSource, Layout, Node, Point, Profile, Rect, Size, SplitSide, SplitterKind, compute_rects,
    };
    use slotmap::Key;

    fn metrics() -> DockMetrics {
        DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        }
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    fn panel_id(layout: &Layout, key: &str) -> crate::PanelId {
        layout
            .panels()
            .iter()
            .find_map(|(id, panel)| (panel.key().as_str() == key).then_some(id))
            .expect("panel exists")
    }

    fn group_id(layout: &Layout, key: &str) -> crate::NodeId {
        layout
            .dock_areas()
            .iter()
            .flat_map(|area| area.columns.iter())
            .flat_map(|column| column.groups.iter().copied())
            .find(|group| {
                matches!(
                    layout.nodes().get(*group),
                    Some(Node::Tabs(tabs))
                        if tabs.panels.iter().any(|panel| layout.panels().get(*panel)
                            .is_some_and(|instance| instance.key().as_str() == key))
                )
            })
            .expect("group exists")
    }

    fn geometry(layout: &Layout, metrics: &DockMetrics) -> crate::LayoutRects {
        compute_rects(layout, Size::new(1280.0, 800.0), metrics)
    }

    #[test]
    fn horizontal_drop_zones_belong_to_column_ends_and_group_splitters() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let first_group = group_id(&layout, "layers");
        let next_group = group_id(&layout, "properties");
        let payload = DropPayload::Panel(panel_id(&layout, "color"));
        let center_payload = DropPayload::Panel(panel_id(&layout, "properties"));
        let rects = geometry(&layout, &metrics);
        let column = ColumnRef::new(DockSide::Right, 0);

        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 31.0), payload, &rects, &metrics),
            Some(DropTarget::Column {
                column,
                zone: DropZone::Top,
            }),
            "top horizontal target is centered on the boundary below the shared column header"
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 43.0), center_payload, &rects, &metrics),
            Some(DropTarget::Column {
                column,
                zone: DropZone::Top,
            }),
            "the widened column-top target remains active up to its lower edge"
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 48.0), center_payload, &rects, &metrics),
            None,
            "a neutral gap separates the column-top target from Center"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1100.0, 54.0),
                center_payload,
                &rects,
                &metrics,
            ),
            Some(DropTarget::Group {
                group: first_group,
                zone: DropZone::Center,
            }),
            "the group's tab strip becomes a center target after the neutral gap"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1100.0, 200.0),
                center_payload,
                &rects,
                &metrics,
            ),
            Some(DropTarget::Group {
                group: first_group,
                zone: DropZone::Center,
            })
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 764.0), payload, &rects, &metrics),
            Some(DropTarget::Group {
                group: next_group,
                zone: DropZone::Center,
            }),
            "the last group's center remains active above the bottom neutral gap"
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 772.0), payload, &rects, &metrics),
            None,
            "a neutral gap separates Center from the column-bottom target"
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 778.0), payload, &rects, &metrics),
            Some(DropTarget::Column {
                column,
                zone: DropZone::Bottom,
            }),
            "the widened bottom target belongs to the column, not its last group"
        );

        let splitter = rects
            .splitters
            .iter()
            .find(|splitter| splitter.kind == SplitterKind::Group)
            .expect("initial layout has a horizontal group splitter");
        let splitter_center = Point::new(
            splitter.bounds.origin.x + splitter.bounds.size.width * 0.5,
            splitter.bounds.origin.y + splitter.bounds.size.height * 0.5,
        );
        assert_eq!(
            resolve_drop(&layout, splitter_center, payload, &rects, &metrics),
            Some(DropTarget::Group {
                group: next_group,
                zone: DropZone::Top,
            }),
            "the center of the visible group splitter resolves to a split target"
        );
        for (offset, target) in [
            (
                -10.0,
                DropTarget::Group {
                    group: first_group,
                    zone: DropZone::Bottom,
                },
            ),
            (
                10.0,
                DropTarget::Group {
                    group: next_group,
                    zone: DropZone::Top,
                },
            ),
        ] {
            assert_eq!(
                resolve_drop(
                    &layout,
                    Point::new(splitter_center.x, splitter_center.y + offset),
                    payload,
                    &rects,
                    &metrics,
                ),
                Some(target),
                "the upper and lower halves of a splitter must resolve to distinct semantic targets"
            );
        }
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(splitter_center.x, splitter_center.y + 14.0),
                payload,
                &rects,
                &metrics,
            ),
            None,
            "a neutral gap separates the split target from the center target"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(splitter_center.x, splitter_center.y + 24.0),
                payload,
                &rects,
                &metrics,
            ),
            Some(DropTarget::Group {
                group: next_group,
                zone: DropZone::Center,
            }),
            "the center target begins again after the neutral gap"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(splitter_center.x, splitter_center.y - 14.0),
                center_payload,
                &rects,
                &metrics,
            ),
            None,
            "the neutral gap is also present above the splitter"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(splitter_center.x, splitter_center.y - 24.0),
                center_payload,
                &rects,
                &metrics,
            ),
            Some(DropTarget::Group {
                group: first_group,
                zone: DropZone::Center,
            }),
            "the previous group's center remains available outside the gap"
        );
        let zones = compute_drop_zones(&layout, &rects, payload, &metrics);
        let split_zone = zones
            .iter()
            .find(|zone| {
                zone.target
                    == (DropTarget::Group {
                        group: next_group,
                        zone: DropZone::Top,
                    })
            })
            .expect("group split target should be advertised");
        assert_eq!(split_zone.bounds.origin.x, splitter.bounds.origin.x);
        assert_eq!(split_zone.bounds.size.width, splitter.bounds.size.width);
        assert_eq!(
            split_zone.bounds.size.height,
            HORIZONTAL_DROP_ZONE_MIN_HEIGHT.max(splitter.bounds.size.height) * 0.5
        );
        assert!(
            (split_zone.bounds.origin.y - splitter_center.y).abs() < 0.001,
            "the lower split hit half starts at the actual splitter center"
        );
        let upper_split_zone = zones
            .iter()
            .find(|zone| {
                zone.target
                    == (DropTarget::Group {
                        group: first_group,
                        zone: DropZone::Bottom,
                    })
            })
            .expect("group bottom target should be advertised");
        assert_eq!(upper_split_zone.bounds.origin.x, splitter.bounds.origin.x);
        assert_eq!(upper_split_zone.bounds.size.width, splitter.bounds.size.width);
        assert!(
            (upper_split_zone.bounds.origin.y + upper_split_zone.bounds.size.height
                - splitter_center.y)
                .abs()
                < 0.001,
            "the upper and lower split zones meet at the splitter without overlapping"
        );

        let top_column_zone = zones
            .iter()
            .find(|zone| {
                zone.target
                    == (DropTarget::Column {
                        column,
                        zone: DropZone::Top,
                    })
            })
            .expect("column top zone should be present");
        assert_eq!(top_column_zone.bounds, rect(1000.0, 20.0, 280.0, 24.0));
        let bottom_column_zone = zones
            .iter()
            .find(|zone| {
                zone.target
                    == (DropTarget::Column {
                        column,
                        zone: DropZone::Bottom,
                    })
            })
            .expect("column bottom zone should be present");
        assert_eq!(bottom_column_zone.bounds, rect(1000.0, 776.0, 280.0, 24.0));
        assert!(
            zones
                .iter()
                .filter(|zone| matches!(
                    &zone.target,
                    DropTarget::Group {
                        zone: DropZone::Top | DropZone::Bottom,
                        ..
                    }
                ))
                .all(|zone| rects.splitters.iter().any(|splitter| {
                    if splitter.kind != SplitterKind::Group
                        || splitter.bounds.origin.x != zone.bounds.origin.x
                        || splitter.bounds.size.width != zone.bounds.size.width
                    {
                        return false;
                    }
                    let splitter_center_y =
                        splitter.bounds.origin.y + splitter.bounds.size.height * 0.5;
                    let zone_bottom = zone.bounds.origin.y + zone.bounds.size.height;
                    // The upper and lower semantic targets meet at the splitter center,
                    // do not overlap, and remain wide enough to activate with a pointer.
                    zone.bounds.origin.y <= splitter_center_y + 0.001
                        && zone_bottom >= splitter_center_y - 0.001
                        && zone.bounds.size.height >= splitter.bounds.size.height
                })),
            "group top/bottom targets must meet at a real splitter"
        );
    }

    #[test]
    fn resolves_external_dock_edges_and_column_edges() {
        let mut metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).unwrap();
        let first_group = group_id(&layout, "layers");
        let first_panel = panel_id(&layout, "layers");
        layout
            .apply(
                DockCommand::SplitGroup {
                    panel: first_panel,
                    group: first_group,
                    side: SplitSide::Right,
                },
                &metrics,
            )
            .expect("second column is allowed");
        let rects = geometry(&layout, &metrics);
        let payload = DropPayload::Panel(panel_id(&layout, "history"));

        assert_eq!(
            resolve_drop(&layout, Point::new(1.0, 300.0), payload, &rects, &metrics),
            Some(DropTarget::DockEdge {
                side: DockSide::Left
            })
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1279.0, 300.0),
                payload,
                &rects,
                &metrics
            ),
            Some(DropTarget::DockEdge {
                side: DockSide::Right
            })
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1000.0, 300.0),
                payload,
                &rects,
                &metrics
            ),
            Some(DropTarget::Column {
                column: ColumnRef::new(DockSide::Right, 0),
                zone: DropZone::Right,
            })
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(725.0, 300.0), payload, &rects, &metrics),
            Some(DropTarget::Column {
                column: ColumnRef::new(DockSide::Right, 0),
                zone: DropZone::Left,
            })
        );

        metrics.max_columns = 1;
        let rects = geometry(&layout, &metrics);
        let zones = compute_drop_zones(&layout, &rects, payload, &metrics);
        assert!(zones.iter().all(|zone| !matches!(
            &zone.target,
            DropTarget::DockEdge { .. }
                | DropTarget::Column {
                    zone: DropZone::Left | DropZone::Right,
                    ..
                }
        )));
        assert!(!matches!(
            resolve_drop(&layout, Point::new(1.0, 300.0), payload, &rects, &metrics),
            Some(
                DropTarget::DockEdge { .. }
                    | DropTarget::Column {
                        zone: DropZone::Left | DropZone::Right,
                        ..
                    }
            )
        ));
    }

    #[test]
    fn hides_four_sided_targets_when_document_or_group_minimums_do_not_fit() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let payload = DropPayload::Panel(panel_id(&layout, "color"));

        let narrow_rects = compute_rects(&layout, Size::new(500.0, 800.0), &metrics);
        assert!(
            narrow_rects.canvas.size.width < metrics.min_document_width,
            "fixture must leave less than the minimum document width"
        );
        let narrow_zones = compute_drop_zones(&layout, &narrow_rects, payload, &metrics);
        assert!(
            narrow_zones.iter().all(|zone| !matches!(
                &zone.target,
                DropTarget::DockEdge { .. }
                    | DropTarget::Column {
                        zone: DropZone::Left | DropZone::Right,
                        ..
                    }
            )),
            "lateral targets must be hidden when they would squeeze Canvas below its minimum"
        );

        let short_rects = compute_rects(&layout, Size::new(1280.0, 150.0), &metrics);
        let short_zones = compute_drop_zones(&layout, &short_rects, payload, &metrics);
        assert!(
            short_zones.iter().all(|zone| !matches!(
                &zone.target,
                DropTarget::Group {
                    zone: DropZone::Top | DropZone::Bottom,
                    ..
                } | DropTarget::Column {
                    zone: DropZone::Top | DropZone::Bottom,
                    ..
                }
            )),
            "vertical split targets must be hidden when group minimum heights do not fit"
        );
    }

    #[test]
    fn tab_reorder_targets_stay_in_source_group_and_follow_horizontal_boundaries() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let group = group_id(&layout, "layers");
        let rects = geometry(&layout, &metrics);
        let external_panel = DropPayload::Panel(panel_id(&layout, "properties"));
        let external_zones = compute_drop_zones(&layout, &rects, external_panel, &metrics);
        assert!(
            !external_zones.iter().any(|zone| {
                matches!(
                    &zone.target,
                    DropTarget::TabInsertion {
                        group: target_group,
                        ..
                    } if *target_group == group
                )
            }),
            "a panel from another group must not create tab insertion targets here"
        );

        let first_tab_panel = DropPayload::Panel(panel_id(&layout, "layers"));
        let first_tab_zones = compute_drop_zones(&layout, &rects, first_tab_panel, &metrics);
        let first_tab_indices: Vec<_> = first_tab_zones
            .iter()
            .filter_map(|zone| match &zone.target {
                DropTarget::TabInsertion {
                    group: target_group,
                    index,
                } if *target_group == group => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(
            first_tab_indices,
            vec![2],
            "no-op boundaries around the dragged tab are omitted"
        );
        assert!(
            first_tab_zones
                .iter()
                .filter_map(|zone| match &zone.target {
                    DropTarget::TabInsertion {
                        group: target_group,
                        ..
                    } => Some(*target_group),
                    _ => None,
                })
                .all(|target_group| target_group == group),
            "reorder targets must stay inside the source group"
        );

        let second_tab_panel = DropPayload::Panel(panel_id(&layout, "color"));
        let second_tab_zones = compute_drop_zones(&layout, &rects, second_tab_panel, &metrics);
        let second_tab_indices: Vec<_> = second_tab_zones
            .iter()
            .filter_map(|zone| match &zone.target {
                DropTarget::TabInsertion {
                    group: target_group,
                    index,
                } if *target_group == group => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(second_tab_indices, vec![0]);

        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1120.0, 45.0),
                external_panel,
                &rects,
                &metrics,
            ),
            None,
            "the top neutral gap must remain empty"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1120.0, 54.0),
                external_panel,
                &rects,
                &metrics,
            ),
            Some(DropTarget::Group {
                group,
                zone: DropZone::Center,
            }),
            "cross-group panel drops use the center target, not a tab insertion target"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1120.0, 70.0),
                external_panel,
                &rects,
                &metrics,
            ),
            Some(DropTarget::Group {
                group,
                zone: DropZone::Center,
            })
        );

        let strip = rects
            .tab_strips
            .iter()
            .find(|strip| strip.group == group)
            .expect("source group has a tab strip");
        let marker_y = strip.bounds.origin.y + strip.bounds.size.height * 0.25;
        let first_header = rects
            .tab_headers
            .iter()
            .find(|header| header.group == group && header.index == 0)
            .unwrap();
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(first_header.bounds.origin.x, marker_y),
                second_tab_panel,
                &rects,
                &metrics,
            ),
            Some(DropTarget::TabInsertion { group, index: 0 }),
            "horizontal reorder works at the normal cursor height"
        );
        let second_header = rects
            .tab_headers
            .iter()
            .find(|header| header.group == group && header.index == 1)
            .unwrap();
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(
                    second_header.bounds.origin.x + second_header.bounds.size.width - 1.0,
                    marker_y,
                ),
                first_tab_panel,
                &rects,
                &metrics,
            ),
            Some(DropTarget::TabInsertion { group, index: 2 }),
        );
    }

    #[test]
    fn floating_group_center_accepts_a_panel_drop() {
        let metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).unwrap();
        let floating_panel = panel_id(&layout, "layers");
        let incoming_panel = panel_id(&layout, "properties");
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(floating_panel),
                    rect: rect(320.0, 200.0, 320.0, 240.0),
                },
                &metrics,
            )
            .expect("panel should float");
        let floating = layout
            .floating_groups()
            .iter()
            .find_map(|(id, group)| group.panels.contains(&floating_panel).then_some(id))
            .unwrap();
        let rects = geometry(&layout, &metrics);

        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(480.0, 320.0),
                DropPayload::Panel(incoming_panel),
                &rects,
                &metrics,
            ),
            Some(DropTarget::FloatingGroup {
                floating,
                zone: DropZone::Center,
            })
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(480.0, 320.0),
                DropPayload::FloatingGroup(floating),
                &rects,
                &metrics,
            ),
            None
        );
    }

    #[test]
    fn floating_panel_can_drop_into_docked_group_center_without_enabling_split_targets() {
        let metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).unwrap();
        let floating_panel = panel_id(&layout, "layers");
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(floating_panel),
                    rect: rect(320.0, 200.0, 320.0, 240.0),
                },
                &metrics,
            )
            .expect("source panel should float");
        let target_group = group_id(&layout, "properties");
        let rects = geometry(&layout, &metrics);
        let bounds = rects
            .groups
            .iter()
            .find(|group| group.group == target_group)
            .expect("dock target group geometry exists")
            .bounds;
        let center = Point::new(
            bounds.origin.x + bounds.size.width * 0.5,
            bounds.origin.y + bounds.size.height * 0.5,
        );
        let payload = DropPayload::Panel(floating_panel);

        assert_eq!(
            resolve_drop(&layout, center, payload, &rects, &metrics),
            Some(DropTarget::Group {
                group: target_group,
                zone: DropZone::Center,
            }),
            "a floating tab should be accepted by the center of an existing dock group"
        );

        if let Some(splitter) = rects
            .splitters
            .iter()
            .find(|splitter| splitter.kind == SplitterKind::Group)
        {
            let split_center = Point::new(
                splitter.bounds.origin.x + splitter.bounds.size.width * 0.5,
                splitter.bounds.origin.y + splitter.bounds.size.height * 0.5,
            );
            assert_ne!(
                resolve_drop(&layout, split_center, payload, &rects, &metrics),
                Some(DropTarget::Group {
                    group: target_group,
                    zone: DropZone::Top,
                }),
                "a floating single tab uses existing-group center drops; whole groups own split drops"
            );
        }
    }

    #[test]
    fn floating_group_drop_zone_excludes_titlebar_edges_and_occluded_canvas() {
        let metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).unwrap();
        let floating_panel = panel_id(&layout, "layers");
        let incoming_panel = panel_id(&layout, "properties");
        let bounds = rect(320.0, 200.0, 320.0, 240.0);
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(floating_panel),
                    rect: bounds,
                },
                &metrics,
            )
            .expect("panel should float");
        let floating = layout
            .floating_groups()
            .iter()
            .find_map(|(id, group)| group.panels.contains(&floating_panel).then_some(id))
            .expect("floating group exists");
        let rects = geometry(&layout, &metrics);
        let payload = DropPayload::Panel(incoming_panel);
        let zones = compute_drop_zones(&layout, &rects, payload, &metrics);

        let floating_zone = zones
            .iter()
            .find(|zone| {
                zone.target
                    == (DropTarget::FloatingGroup {
                        floating,
                        zone: DropZone::Center,
                    })
            })
            .expect("visible center target exists");
        assert_eq!(floating_zone.bounds, rect(344.0, 232.0, 272.0, 184.0));

        let center = Point::new(480.0, 320.0);
        assert_eq!(
            resolve_drop(&layout, center, payload, &rects, &metrics),
            Some(DropTarget::FloatingGroup {
                floating,
                zone: DropZone::Center,
            })
        );
        for blocked_point in [
            Point::new(480.0, 210.0), // title bar
            Point::new(322.0, 320.0), // left resize edge
            Point::new(480.0, 438.0), // bottom resize edge
        ] {
            assert_eq!(
                resolve_drop(&layout, blocked_point, payload, &rects, &metrics),
                None,
                "non-center point {blocked_point:?} must not become a tab target or fall through"
            );
            assert!(
                !zones
                    .iter()
                    .any(|zone| contains(zone.bounds, blocked_point)),
                "overlay must not advertise a drop zone at blocked point {blocked_point:?}"
            );
        }

        assert!(
            zones.iter().any(|zone| {
                contains(zone.bounds, Point::new(100.0, 100.0))
                    && matches!(&zone.target, DropTarget::FreeArea { .. })
            }),
            "visible Canvas area outside the floating window should keep its free-area target"
        );
        assert!(
            zones.iter().any(|zone| {
                contains(zone.bounds, Point::new(700.0, 300.0))
                    && matches!(&zone.target, DropTarget::FreeArea { .. })
            }),
            "visible Canvas area beside the floating window should keep its free-area target"
        );

        let zones_at_center: Vec<_> = zones
            .iter()
            .filter(|zone| contains(zone.bounds, center))
            .collect();
        assert_eq!(zones_at_center.len(), 1);
        assert_eq!(
            zones_at_center[0].target,
            DropTarget::FloatingGroup {
                floating,
                zone: DropZone::Center,
            }
        );
    }

    #[test]
    fn dropping_on_a_frontmost_floating_group_does_not_fall_through() {
        let metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).unwrap();
        let first_panel = panel_id(&layout, "layers");
        let second_panel = panel_id(&layout, "properties");
        let bounds = rect(320.0, 200.0, 320.0, 240.0);
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(first_panel),
                    rect: bounds,
                },
                &metrics,
            )
            .expect("first panel should float");
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(second_panel),
                    rect: bounds,
                },
                &metrics,
            )
            .expect("second panel should float");

        let mut floating: Vec<_> = layout.floating_groups().iter().collect();
        floating.sort_by_key(|(_, group)| group.z_order);
        let bottom = floating[0].0;
        let top = floating[1].0;
        let rects = geometry(&layout, &metrics);
        assert_ne!(bottom, top);

        let payload = DropPayload::FloatingGroup(top);
        assert_eq!(
            resolve_drop(&layout, Point::new(480.0, 320.0), payload, &rects, &metrics,),
            None
        );
        assert!(
            compute_drop_zones(&layout, &rects, payload, &metrics)
                .iter()
                .all(|zone| !contains(zone.bounds, Point::new(480.0, 320.0))),
            "the invalid frontmost self-target must occlude every lower target in the overlay"
        );
    }

    #[test]
    fn free_canvas_detaches_panels_and_whole_groups_inside_the_client_viewport() {
        let metrics = metrics();

        // Detaching one tab clamps its floating bounds at the client edge and commits
        // only after the drag session releases the resulting FreeArea target.
        let mut panel_layout = Layout::initial_photoshop(&metrics).unwrap();
        let panel = panel_id(&panel_layout, "layers");
        let panel_rects = geometry(&panel_layout, &metrics);
        let panel_target = resolve_drop(
            &panel_layout,
            Point::new(30.0, 10.0),
            DropPayload::Panel(panel),
            &panel_rects,
            &metrics,
        )
        .expect("free Canvas area should accept a single panel detach");
        let DropTarget::FreeArea { bounds: panel_bounds } = panel_target else {
            panic!("Canvas drop must create a floating group, not a dock target");
        };
        assert_eq!(panel_bounds.origin, Point::new(0.0, 0.0));
        assert!(panel_bounds.origin.x >= 0.0 && panel_bounds.origin.y >= 0.0);
        assert!(panel_bounds.origin.x + panel_bounds.size.width <= 1280.0);
        assert!(panel_bounds.origin.y + panel_bounds.size.height <= 800.0);

        panel_layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel),
                    rect: panel_bounds,
                },
                &metrics,
            )
            .expect("release should detach a single panel");
        let floating_panel = panel_layout
            .floating_groups()
            .values()
            .next()
            .expect("detached panel should be visible in a floating group");
        assert_eq!(floating_panel.panels, vec![panel]);
        assert_eq!(floating_panel.active_index, 0);
        assert!(panel_layout.validate(&metrics).is_ok());

        // The blank group capture area sends a Group payload. All tabs and the
        // active tab survive detachment, and both near-edge placements are clamped.
        let mut group_layout = Layout::initial_photoshop(&metrics).unwrap();
        let group = group_id(&group_layout, "layers");
        group_layout
            .apply(DockCommand::ActivateTab { group, index: 1 }, &metrics)
            .expect("test group can activate its second tab");
        let original_panels = match group_layout.nodes().get(group).unwrap() {
            Node::Tabs(tabs) => {
                assert_eq!(tabs.active_index, 1);
                tabs.panels.clone()
            }
            Node::Split(_) => panic!("Photoshop group should be Tabs"),
        };
        let group_rects = geometry(&group_layout, &metrics);
        let group_payload = DropPayload::Group(group);
        let DropTarget::FreeArea { bounds: group_bounds } = resolve_drop(
            &group_layout,
            Point::new(30.0, 10.0),
            group_payload,
            &group_rects,
            &metrics,
        )
        .expect("free Canvas area should accept a whole-group detach") else {
            panic!("whole-group Canvas drop must resolve to FreeArea");
        };
        assert_eq!(group_bounds.origin, Point::new(0.0, 0.0));
        assert!(group_bounds.origin.x + group_bounds.size.width <= 1280.0);
        assert!(group_bounds.origin.y + group_bounds.size.height <= 800.0);

        let DropTarget::FreeArea { bounds: lower_bounds } = resolve_drop(
            &group_layout,
            Point::new(960.0, 790.0),
            group_payload,
            &group_rects,
            &metrics,
        )
        .expect("Canvas remains a valid target near its lower edge") else {
            panic!("lower-edge group detach must resolve to FreeArea");
        };
        assert!(lower_bounds.origin.x >= 0.0 && lower_bounds.origin.y >= 0.0);
        assert!(lower_bounds.origin.x + lower_bounds.size.width <= 1280.0);
        assert!(lower_bounds.origin.y + lower_bounds.size.height <= 800.0);

        group_layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Group(group),
                    rect: group_bounds,
                },
                &metrics,
            )
            .expect("release should detach the complete group");
        assert!(group_layout.nodes().get(group).is_none());
        let floating_group = group_layout
            .floating_groups()
            .values()
            .next()
            .expect("detached group should remain inside the main window");
        assert_eq!(floating_group.panels, original_panels);
        assert_eq!(floating_group.active_index, 1);
        assert_eq!(floating_group.bounds, group_bounds);
        assert!(group_layout.validate(&metrics).is_ok());
    }

    #[test]
    fn free_canvas_area_creates_a_cursor_centered_floating_rect() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let payload = DropPayload::Panel(panel_id(&layout, "layers"));
        let rects = geometry(&layout, &metrics);

        let Some(DropTarget::FreeArea { bounds }) =
            resolve_drop(&layout, Point::new(400.0, 300.0), payload, &rects, &metrics)
        else {
            panic!("empty Canvas surface should produce a floating destination");
        };
        assert_eq!(bounds.size.width, 280.0);
        assert!(bounds.size.height >= metrics.column_header_height + metrics.tab_strip_height);
        assert_eq!(
            bounds.origin,
            Point::new(
                400.0 - bounds.size.width * 0.5,
                300.0 - bounds.size.height * 0.5
            )
        );
    }

    #[test]
    fn free_area_zone_is_not_advertised_when_minimum_floating_size_cannot_fit() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let payload = DropPayload::Panel(panel_id(&layout, "layers"));
        let rects = compute_rects(&layout, Size::new(1280.0, 100.0), &metrics);

        let zones = compute_drop_zones(&layout, &rects, payload, &metrics);
        assert!(
            zones
                .iter()
                .all(|zone| !matches!(&zone.target, DropTarget::FreeArea { .. })),
            "FreeArea is invalid when the minimum floating height exceeds the viewport"
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(400.0, 50.0), payload, &rects, &metrics),
            None
        );

        let group_payload = DropPayload::Group(group_id(&layout, "layers"));
        let group_zones = compute_drop_zones(&layout, &rects, group_payload, &metrics);
        assert!(
            group_zones
                .iter()
                .all(|zone| !matches!(&zone.target, DropTarget::FreeArea { .. })),
            "a whole group needs room for its title bar, tab strip, and minimum content height"
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(400.0, 50.0),
                group_payload,
                &rects,
                &metrics,
            ),
            None
        );
    }

    #[test]
    fn self_drop_missing_nodes_and_side_splits_inside_groups_are_rejected() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let group = group_id(&layout, "layers");
        let rects = geometry(&layout, &metrics);

        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1100.0, 200.0),
                DropPayload::Group(group),
                &rects,
                &metrics,
            ),
            None
        );
        assert!(
            !compute_drop_zones(
                &layout,
                &rects,
                DropPayload::Panel(panel_id(&layout, "properties")),
                &metrics,
            )
            .iter()
            .any(|zone| matches!(
                &zone.target,
                DropTarget::Group {
                    zone: DropZone::Left | DropZone::Right,
                    ..
                }
            ))
        );

        let missing_group = crate::NodeId::null();
        let payload = DropPayload::Panel(panel_id(&layout, "properties"));
        let rects_with_missing_group = crate::LayoutRects {
            groups: vec![crate::GroupRect {
                group: missing_group,
                bounds: rect(1000.0, 100.0, 280.0, 300.0),
            }],
            ..rects.clone()
        };
        assert!(
            !compute_drop_zones(&layout, &rects_with_missing_group, payload, &metrics,)
                .iter()
                .any(|zone| matches!(
                    &zone.target,
                    DropTarget::Group { group, .. } if *group == missing_group
                ))
        );

        let empty = Layout::new(Profile::Photoshop, crate::CanvasState::default());
        assert!(compute_drop_zones(&empty, &rects, payload, &metrics).is_empty());
        assert!(
            compute_drop_zones(&layout, &rects, DropPayload::Group(missing_group), &metrics,)
                .is_empty()
        );
    }

    #[test]
    fn invalid_or_non_finite_points_do_not_resolve() {
        let metrics = metrics();
        let mut layout = Layout::initial_photoshop(&metrics).unwrap();
        let rects = geometry(&layout, &metrics);
        let payload = DropPayload::Panel(panel_id(&layout, "layers"));

        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(f32::NAN, 200.0),
                payload,
                &rects,
                &metrics
            ),
            None
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(200.0, f32::INFINITY),
                payload,
                &rects,
                &metrics
            ),
            None
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(640.0, 400.0),
                DropPayload::Panel(crate::PanelId::null()),
                &rects,
                &metrics
            ),
            None
        );

        let panel = panel_id(&layout, "layers");
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel),
                    rect: rect(320.0, 200.0, 320.0, 240.0),
                },
                &metrics,
            )
            .expect("panel should float");
        let floating_id = layout
            .floating_groups()
            .iter()
            .find_map(|(id, floating)| floating.panels.contains(&panel).then_some(id))
            .expect("floating group exists");
        let floating_rects = geometry(&layout, &metrics);
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(480.0, 320.0),
                DropPayload::Panel(crate::PanelId::null()),
                &floating_rects,
                &metrics,
            ),
            None
        );
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1100.0, 200.0),
                DropPayload::Panel(panel),
                &floating_rects,
                &metrics,
            ),
            Some(DropTarget::Group {
                group: group_id(&layout, "color"),
                zone: DropZone::Center,
            }),
            "a valid finite drop from a floating panel onto a dock group is accepted"
        );
        assert!(layout.floating_groups().contains_key(floating_id));
    }
}
