use crate::command::{ColumnRef, DropTarget, DropZone};
use crate::geometry::{
    DockMetrics, DropZoneRect, GroupRect, LayoutRects, Point, Rect, Size,
};
use crate::ids::{FloatingGroupId, NodeId, PanelId};
use crate::model::{DockSide, Layout, Node};

use super::DropPayload;

/// Build data-only rectangles for the docking overlay.
///
/// The vector is ordered by hit-test priority: floating groups (front to back),
/// external dock edges, column edges, tab insertion positions, group edges/centers,
/// and finally the free Canvas area. The overlay can paint these rectangles without
/// depending on GPUI or other rendering code.
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

    // A floating window occludes dock targets beneath it. The geometry list is
    // bottom-to-top, so visit it in reverse paint order.
    for floating_rect in rects.floating_groups.iter().rev() {
        let Some(floating) = layout.floating_groups().get(floating_rect.floating) else {
            continue;
        };
        if !valid_target(
            layout,
            payload,
            &DropTarget::FloatingGroup {
                floating: floating_rect.floating,
                zone: DropZone::Center,
            },
            metrics,
        ) {
            continue;
        }
        if floating.panels.is_empty() || !valid_rect(floating_rect.bounds) {
            continue;
        }
        push_zone(
            &mut zones,
            DropTarget::FloatingGroup {
                floating: floating_rect.floating,
                zone: DropZone::Center,
            },
            floating_rect.bounds,
        );
    }

    let can_add_column = column_count(layout) < metrics.max_columns;
    let outer_zone_width = finite_positive(metrics.edge_drop_zone_width);
    if can_add_column && outer_zone_width > 0.0 {
        push_zone(
            &mut zones,
            DropTarget::DockEdge { side: DockSide::Left },
            Rect::new(
                Point::new(viewport.origin.x, viewport.origin.y),
                Size::new(outer_zone_width.min(viewport.size.width), viewport.size.height),
            ),
        );
        push_zone(
            &mut zones,
            DropTarget::DockEdge { side: DockSide::Right },
            Rect::new(
                Point::new(
                    viewport.origin.x + viewport.size.width
                        - outer_zone_width.min(viewport.size.width),
                    viewport.origin.y,
                ),
                Size::new(outer_zone_width.min(viewport.size.width), viewport.size.height),
            ),
        );
    }

    let edge_width = finite_positive(metrics.drop_edge_zone).min(viewport.size.width);
    if can_add_column && edge_width > 0.0 {
        append_column_edge_zones(layout, rects, &mut zones, viewport, edge_width);
    }

    // Tab insertion zones precede group center/edge zones so dropping between two
    // labels can reorder or insert a tab rather than create a neighboring group.
    if matches!(payload, DropPayload::Panel(_) | DropPayload::FloatingGroup(_)) {
        append_tab_insertion_zones(layout, rects, &mut zones, metrics);
    }

    append_group_zones(layout, rects, payload, &mut zones, metrics);

    // Canvas is protected as a docking destination, but its unoccupied surface is a
    // valid place to release a docked panel/group and create a separate floating group.
    if is_docked_payload(layout, payload) && valid_rect(rects.canvas) {
        push_zone(
            &mut zones,
            DropTarget::FreeArea { bounds: Rect::default() },
            rects.canvas,
        );
    }

    zones
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
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }

    let zones = compute_drop_zones(layout, rects, payload, metrics);
    let zone = zones.iter().find(|zone| contains(zone.bounds, point))?;
    match &zone.target {
        DropTarget::FreeArea { .. } => {
            let bounds = floating_bounds_at_point(layout, rects, payload, point, metrics)?;
            Some(DropTarget::FreeArea { bounds })
        }
        target if valid_target(layout, payload, target, metrics) => Some(target.clone()),
        _ => None,
    }
}

fn append_column_edge_zones(
    layout: &Layout,
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
            let left = (boundary_x - zone_width * 0.5)
                .max(viewport.origin.x);
            let right = (boundary_x + zone_width * 0.5)
                .min(viewport.origin.x + viewport.size.width);
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

    // Empty dock areas still have a valid outer DockEdge zone above. Column targets
    // are emitted only for concrete columns present in the model.
    let _ = layout;
}

fn append_tab_insertion_zones(
    layout: &Layout,
    rects: &LayoutRects,
    zones: &mut Vec<DropZoneRect>,
    metrics: &DockMetrics,
) {
    let requested_width = finite_positive(metrics.drop_edge_zone);
    if requested_width <= 0.0 {
        return;
    }

    for group_rect in &rects.groups {
        let Some(Node::Tabs(tabs)) = layout.nodes().get(group_rect.group) else {
            continue;
        };
        if !is_docked_group(layout, group_rect.group) || tabs.panels.len() < 2 {
            continue;
        }
        let Some(strip) = rects.tab_strips.iter().find(|strip| strip.group == group_rect.group)
        else {
            continue;
        };
        if !valid_rect(strip.bounds) {
            continue;
        }

        let mut headers: Vec<_> = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == group_rect.group)
            .collect();
        headers.sort_by_key(|header| header.index);
        for pair in headers.windows(2) {
            let previous = pair[0];
            let next = pair[1];
            if next.index != previous.index + 1 {
                continue;
            }
            let boundary_x = (previous.bounds.origin.x + previous.bounds.size.width
                + next.bounds.origin.x)
                * 0.5;
            let width = requested_width
                .min(previous.bounds.size.width)
                .min(next.bounds.size.width);
            if width <= 0.0 {
                continue;
            }

            let target = DropTarget::TabInsertion {
                group: group_rect.group,
                index: next.index,
            };
            if valid_target(layout, current_payload_placeholder(layout), &target, metrics) {
                push_zone(
                    zones,
                    target,
                    Rect::new(
                        Point::new(boundary_x - width * 0.5, strip.bounds.origin.y),
                        Size::new(width, strip.bounds.size.height),
                    ),
                );
            }
        }
    }
}

fn current_payload_placeholder(layout: &Layout) -> DropPayload {
    // Tab insertion zones are only appended by compute_drop_zones for Panel or
    // FloatingGroup payloads. A real payload is checked again in resolve_drop.
    if let Some((id, _)) = layout.panels().iter().next() {
        DropPayload::Panel(id)
    } else if let Some((id, _)) = layout.floating_groups().iter().next() {
        DropPayload::FloatingGroup(id)
    } else {
        DropPayload::Panel(PanelId::null())
    }
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
        let vertical_edge = edge.min(bounds.size.height / 3.0);

        // Top/bottom zones span the group width. Column-edge zones have higher
        // priority and therefore win where these strips overlap.
        let zone_width = bounds.size.width;
        let top_bounds = Rect::new(bounds.origin, Size::new(zone_width, vertical_edge));
        let bottom_bounds = Rect::new(
            Point::new(bounds.origin.x, bounds.origin.y + bounds.size.height - vertical_edge),
            Size::new(zone_width, vertical_edge),
        );
        for (zone, zone_bounds) in [
            (DropZone::Top, top_bounds),
            (DropZone::Bottom, bottom_bounds),
        ] {
            let target = DropTarget::Group {
                group: group_rect.group,
                zone,
            };
            if valid_target(layout, payload, &target, metrics) {
                push_zone(zones, target, zone_bounds);
            }
        }

        let center_bounds = Rect::new(
            Point::new(
                bounds.origin.x + horizontal_edge,
                bounds.origin.y + vertical_edge,
            ),
            Size::new(
                (bounds.size.width - horizontal_edge * 2.0).max(0.0),
                (bounds.size.height - vertical_edge * 2.0).max(0.0),
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
            if !is_docked_group(layout, *group)
                || !matches!(layout.nodes().get(*group), Some(Node::Tabs(_)))
                || matches!(payload, DropPayload::Group(source) if source == *group)
            {
                return false;
            }
            match zone {
                DropZone::Center | DropZone::Top | DropZone::Bottom => {}
                DropZone::Left | DropZone::Right => return false,
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
            let Some(Node::Tabs(tabs)) = layout.nodes().get(*group) else {
                return false;
            };
            if !is_docked_group(layout, *group) || *index == 0 || *index >= tabs.panels.len() {
                return false;
            }
            if matches!(payload, DropPayload::Group(_)) {
                return false;
            }
            if let DropPayload::FloatingGroup(source) = payload {
                if layout
                    .floating_groups()
                    .get(source)
                    .is_none_or(|floating| floating.panels.is_empty())
                {
                    return false;
                }
            }
            true
        }
        DropTarget::Column { column, zone } => {
            column_exists(layout, *column)
                && matches!(zone, DropZone::Left | DropZone::Right)
                && can_add_column(layout, metrics)
                && !matches!(payload, DropPayload::Group(_))
        }
        DropTarget::DockEdge { .. } => {
            can_add_column(layout, metrics) && !matches!(payload, DropPayload::Group(_))
        }
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
    layout
        .dock_areas()
        .iter()
        .any(|area| area.columns.iter().any(|column| column.groups.contains(&group)))
}

fn column_exists(layout: &Layout, column: ColumnRef) -> bool {
    layout
        .dock_areas()
        .iter()
        .find(|area| area.side == column.side)
        .is_some_and(|area| column.index < area.columns.len())
}

fn can_add_column(layout: &Layout, metrics: &DockMetrics) -> bool {
    column_count(layout) < metrics.max_columns
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
            let minimum_content_height = finite_positive(metrics.min_group_height)
                .max(finite_positive(minimum.height));
            let width = finite_positive(metrics.default_column_width)
                .max(finite_positive(metrics.min_column_width))
                .max(finite_positive(preferred.width))
                .max(finite_positive(minimum.width));
            let minimum_width = finite_positive(metrics.min_column_width)
                .max(finite_positive(minimum.width));
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
            let bounds = rects.groups.iter().find(|group| group.group == group_id)?.bounds;
            let mut minimum_width = finite_positive(metrics.min_column_width);
            let mut minimum_content_height = finite_positive(metrics.min_group_height);
            for panel_id in &tabs.panels {
                let constraints = layout.panels().get(*panel_id)?.constraints();
                minimum_width = minimum_width.max(finite_positive(constraints.min_size().width));
                minimum_content_height =
                    minimum_content_height.max(finite_positive(constraints.min_size().height));
            }
            let width = bounds.size.width
                .max(finite_positive(metrics.default_column_width))
                .max(minimum_width);
            let height = bounds.size.height + title_height;
            Some((
                width,
                height,
                minimum_width,
                minimum_content_height + title_height,
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
    use super::{compute_drop_zones, resolve_drop};
    use crate::{
        compute_rects, ColumnRef, DockCommand, DockMetrics, DockSide, DropPayload, DropTarget,
        DropZone, FloatSource, Layout, Node, Point, Profile, Rect, Size, SplitSide,
    };

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
    fn resolves_group_center_and_top_bottom_zones() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let group = group_id(&layout, "layers");
        let payload = DropPayload::Panel(panel_id(&layout, "properties"));
        let rects = geometry(&layout, &metrics);

        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 200.0), payload, &rects, &metrics),
            Some(DropTarget::Group {
                group,
                zone: DropZone::Center,
            })
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 40.0), payload, &rects, &metrics),
            Some(DropTarget::Group {
                group,
                zone: DropZone::Top,
            })
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1100.0, 400.0), payload, &rects, &metrics),
            Some(DropTarget::Group {
                group,
                zone: DropZone::Bottom,
            })
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
            Some(DropTarget::DockEdge { side: DockSide::Left })
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(1000.0, 300.0), payload, &rects, &metrics),
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
            zone.target,
            DropTarget::DockEdge { .. } | DropTarget::Column { .. }
        )));
        assert_eq!(
            resolve_drop(&layout, Point::new(1.0, 300.0), payload, &rects, &metrics),
            None
        );
    }

    #[test]
    fn tab_strip_boundaries_resolve_to_insertion_positions() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let group = group_id(&layout, "layers");
        let payload = DropPayload::Panel(panel_id(&layout, "properties"));
        let rects = geometry(&layout, &metrics);

        assert_eq!(
            resolve_drop(&layout, Point::new(1140.0, 45.0), payload, &rects, &metrics),
            Some(DropTarget::TabInsertion { group, index: 1 })
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
            Point::new(400.0 - bounds.size.width * 0.5, 300.0 - bounds.size.height * 0.5)
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
        assert!(!compute_drop_zones(
            &layout,
            &rects,
            DropPayload::Panel(panel_id(&layout, "properties")),
            &metrics,
        )
        .iter()
        .any(|zone| matches!(
            zone.target,
            DropTarget::Group { zone: DropZone::Left | DropZone::Right, .. }
        )));

        let missing_group = crate::NodeId::null();
        let payload = DropPayload::Panel(panel_id(&layout, "properties"));
        assert_eq!(
            resolve_drop(
                &layout,
                Point::new(1100.0, 200.0),
                payload,
                &crate::LayoutRects {
                    groups: vec![crate::GroupRect {
                        group: missing_group,
                        bounds: rect(1000.0, 100.0, 280.0, 300.0),
                    }],
                    ..rects.clone()
                },
                &metrics,
            ),
            Some(DropTarget::Group {
                group,
                zone: DropZone::Center,
            })
        );
        let mut empty = Layout::new(Profile::Photoshop, crate::CanvasState::default());
        assert!(compute_drop_zones(&empty, &rects, payload, &metrics).is_empty());
        empty = layout.clone();
        assert!(compute_drop_zones(
            &empty,
            &rects,
            DropPayload::Group(missing_group),
            &metrics,
        )
        .is_empty());
    }

    #[test]
    fn invalid_or_non_finite_points_do_not_resolve() {
        let metrics = metrics();
        let layout = Layout::initial_photoshop(&metrics).unwrap();
        let rects = geometry(&layout, &metrics);
        let payload = DropPayload::Panel(panel_id(&layout, "layers"));

        assert_eq!(
            resolve_drop(&layout, Point::new(f32::NAN, 200.0), payload, &rects, &metrics),
            None
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(200.0, f32::INFINITY), payload, &rects, &metrics),
            None
        );
        assert_eq!(
            resolve_drop(&layout, Point::new(640.0, 400.0), DropPayload::Panel(crate::PanelId::null()), &rects, &metrics),
            None
        );
    }
}
