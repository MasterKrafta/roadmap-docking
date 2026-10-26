use std::collections::HashSet;

use crate::{
    ComputeRectsError, DockMetrics, DropTarget, DropZone, HitResult, Layout, LayoutRects, Node,
    FlyoutRect, NodeId, PanelId, Point, Rect, ResizeEdge, Size,
};

pub(super) fn flyout_resize_hit_rect(
    flyout: FlyoutRect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let half_zone = metrics.splitter_hit_zone * 0.5;
    let edge_x = match flyout.resize_edge {
        ResizeEdge::Left => flyout.rect.origin.x,
        ResizeEdge::Right => flyout.rect.origin.x + flyout.rect.size.width,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let rect = Rect::new(
        Point::new(edge_x - half_zone, flyout.rect.origin.y),
        Size::new(metrics.splitter_hit_zone, flyout.rect.size.height),
    );
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(rect)
}

pub(super) fn contains(rect: Rect, point: Point) -> Result<bool, ComputeRectsError> {
    rect.contains(point).map_err(ComputeRectsError::Geometry)
}

pub(super) fn hit_layout_geometry(
    layout: &Layout,
    point: Point,
    rects: &LayoutRects,
    metrics: &DockMetrics,
    scope_nodes: &HashSet<NodeId>,
    scope_panels: &HashSet<PanelId>,
    include_only: bool,
) -> Result<Option<HitResult>, ComputeRectsError> {
    // Splitters win over the column/panel areas they overlap. Reversing the
    // deterministic geometry order gives nested/outer boundaries a stable tie.
    for splitter in rects.splitters.iter().rev() {
        if node_in_scope(splitter.owner, scope_nodes, include_only)
            && contains(splitter.rect, point)?
        {
            return Ok(Some(HitResult::Splitter {
                owner: splitter.owner,
                boundary_index: splitter.boundary_index,
                axis: splitter.axis,
            }));
        }
    }

    // Overflow buttons are explicit interactive geometry and take priority
    // over ordinary tab/header fallback areas in their strip.
    for overflow in rects.tab_overflows.iter().rev() {
        if node_in_scope(overflow.group, scope_nodes, include_only)
            && contains(overflow.button_rect, point)?
        {
            return Ok(Some(HitResult::TabOverflowButton {
                group: overflow.group,
            }));
        }
    }

    // Drop zones exist only in a drag snapshot. They beat ordinary group/header
    // and tab hits, while splitters and explicit overflow buttons retain priority.
    // Use the same explicit priority as resolve_drop when zones overlap.
    let mut best_drop_zone = None;
    for (index, zone) in rects.drop_zones.iter().enumerate().rev() {
        if !target_in_scope(zone.target, scope_nodes, include_only) || !contains(zone.rect, point)?
        {
            continue;
        }
        let priority = drop_zone_priority(zone.zone);
        let replace = best_drop_zone
            .as_ref()
            .map(|(best_priority, _, _)| priority < *best_priority)
            .unwrap_or(true);
        if replace {
            best_drop_zone = Some((priority, index, zone));
        }
    }
    if let Some((_, _, zone)) = best_drop_zone {
        return Ok(Some(HitResult::DropZone {
            target: zone.target,
            zone: zone.zone,
        }));
    }

    // A GroupRect's upper header band is the non-floating group/column header.
    // Collapsed groups have no separate header: their tab strip starts at y=0.
    for group in rects.groups.iter().rev() {
        if !node_in_scope(group.group, scope_nodes, include_only) {
            continue;
        }
        if !rects
            .tab_strips
            .iter()
            .any(|strip| strip.group == group.group)
        {
            continue;
        }
        if matches!(layout.node(group.group), Some(Node::Tabs(tabs)) if tabs.collapsed) {
            continue;
        }
        let header_height = metrics.header_height.min(group.rect.size.height);
        let header = Rect::new(
            group.rect.origin,
            crate::Size::new(group.rect.size.width, header_height),
        );
        if contains(header, point)? {
            return Ok(Some(HitResult::Group { group: group.group }));
        }
    }

    for tab in rects.tab_headers.iter().rev() {
        if node_in_scope(tab.group, scope_nodes, include_only) && contains(tab.rect, point)? {
            return Ok(Some(HitResult::TabHeader {
                group: tab.group,
                panel: tab.panel,
                index: tab.index,
            }));
        }
    }

    for panel in rects.panels.iter().rev() {
        if panel_in_scope(panel.panel, scope_panels, include_only) && contains(panel.rect, point)? {
            return Ok(Some(HitResult::Panel { panel: panel.panel }));
        }
    }

    for strip in rects.tab_strips.iter().rev() {
        if node_in_scope(strip.group, scope_nodes, include_only) && contains(strip.rect, point)? {
            return Ok(Some(HitResult::TabStrip { group: strip.group }));
        }
    }

    for group in rects.groups.iter().rev() {
        if node_in_scope(group.group, scope_nodes, include_only) && contains(group.rect, point)? {
            return Ok(Some(HitResult::Group { group: group.group }));
        }
    }

    for column in rects.columns.iter().rev() {
        if node_in_scope(column.column, scope_nodes, include_only) && contains(column.rect, point)?
        {
            return Ok(Some(HitResult::Column {
                column: column.column,
            }));
        }
    }

    Ok(None)
}

fn node_in_scope(node: NodeId, scope_nodes: &HashSet<NodeId>, include_only: bool) -> bool {
    scope_nodes.contains(&node) == include_only
}

fn panel_in_scope(panel: PanelId, scope_panels: &HashSet<PanelId>, include_only: bool) -> bool {
    scope_panels.contains(&panel) == include_only
}

fn target_in_scope(target: DropTarget, scope_nodes: &HashSet<NodeId>, include_only: bool) -> bool {
    let target_node = match target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => Some(group),
        DropTarget::ColumnSlot { column, .. } | DropTarget::RailSlot { column, .. } => Some(column),
        DropTarget::MainColumn { .. } | DropTarget::FreeArea => {
            return !include_only;
        }
    };
    target_node.is_some_and(|node| node_in_scope(node, scope_nodes, include_only))
}

fn drop_zone_priority(zone: DropZone) -> u8 {
    match zone {
        DropZone::RailInsertion => 0,
        DropZone::TabInsertion => 1,
        DropZone::DockEdge => 2,
        DropZone::ColumnLeft | DropZone::ColumnRight => 3,
        DropZone::ColumnEnd => 4,
        DropZone::FloatingTop | DropZone::FloatingBottom => 5,
        DropZone::Top | DropZone::Bottom => 6,
        DropZone::Left | DropZone::Right => 7,
        DropZone::FloatingCenter => 8,
        DropZone::Center => 9,
        DropZone::FreeArea => 10,
    }
}
