//! Thin adapter from a completed pointer gesture to dock-core's semantic drop action.
//!
//! Core owns the source/target validation and the mapping to a single DockCommand.
//! Keep that policy in one place instead of duplicating its decision table in GPUI.

use dock_core::{
    CommandError, DockMetrics, DragDropAction, DragDropRequest, DragHandle, DragPayload,
    DropTarget, Layout, LayoutRects, Node, Point, Rect,
};

/// Resolves a released drag without mutating the layout.
///
/// `DragDropAction::Command` contains the one command to commit through
/// `DockController` history; `NoOp` deliberately produces no history entry.
pub(super) fn resolve_drop(
    layout: &Layout,
    request: &DragDropRequest,
    metrics: &DockMetrics,
) -> Result<DragDropAction, CommandError> {
    layout.resolve_drag_drop(request, metrics)
}

/// Supplies bounds when a valid drop destination asks Core to float the source.
pub(super) fn supply_floating_bounds(
    layout: &Layout,
    rects: &LayoutRects,
    metrics: &DockMetrics,
    viewport: Rect,
    request: &mut DragDropRequest,
) -> Result<(), CommandError> {
    if request.target != Some(DropTarget::FreeArea) {
        return Ok(());
    }

    let source_rect = source_rect_for_request(layout, rects, request)?;
    let grab_offset = Point::new(
        request.origin.x - source_rect.origin.x,
        request.origin.y - source_rect.origin.y,
    );
    request.floating_bounds = Some(super::floating_policy::initial_float_bounds(
        source_rect,
        grab_offset,
        request.point,
        viewport,
        metrics,
    )?);
    Ok(())
}

fn source_rect_for_request(
    layout: &Layout,
    rects: &LayoutRects,
    request: &DragDropRequest,
) -> Result<Rect, CommandError> {
    let rect = match (request.handle, request.payload) {
        (DragHandle::FlyoutPanel, DragPayload::Panel { panel }) => rects
            .flyout
            .filter(|flyout| flyout.panel == panel)
            .map(|flyout| flyout.rect)
            .or_else(|| panel_group_rect(layout, rects, panel))
            .or_else(|| {
                rects
                    .panels
                    .iter()
                    .find(|panel_rect| panel_rect.panel == panel)
                    .map(|panel_rect| panel_rect.rect)
            }),
        (DragHandle::PanelTab, DragPayload::Panel { panel }) => {
            panel_group_rect(layout, rects, panel)
                .or_else(|| {
                    rects
                        .tab_headers
                        .iter()
                        .find(|header| header.panel == panel)
                        .map(|header| header.rect)
                })
                .or_else(|| {
                    rects
                        .panels
                        .iter()
                        .find(|panel_rect| panel_rect.panel == panel)
                        .map(|panel_rect| panel_rect.rect)
                })
        }
        (DragHandle::GroupGrip, DragPayload::Group { group }) => rects
            .groups
            .iter()
            .find(|group_rect| group_rect.group == group)
            .map(|group_rect| group_rect.rect),
        _ => None,
    };

    rect.ok_or(CommandError::InvalidTarget {
        reason: "could not find drag source geometry for floating bounds",
    })
}

fn panel_group_rect(
    layout: &Layout,
    rects: &LayoutRects,
    panel: dock_core::PanelId,
) -> Option<Rect> {
    let panel_node = layout.nodes().find_map(|(node_id, node)| match node {
        Node::Panel(value) if value.panel_id == panel => Some(node_id),
        _ => None,
    })?;
    let group = layout.nodes().find_map(|(node_id, node)| match node {
        Node::Tabs(tabs) if tabs.children.contains(&panel_node) => Some(node_id),
        _ => None,
    })?;
    rects
        .groups
        .iter()
        .find(|group_rect| group_rect.group == group)
        .map(|group_rect| group_rect.rect)
}
