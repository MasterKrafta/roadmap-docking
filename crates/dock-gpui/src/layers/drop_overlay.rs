use std::collections::HashMap;

use dock_core::{DragHandle, DragPayload, Node, NodeId, Point, Rect, Size};
use gpui::{
    AnyElement, IntoElement, ParentElement, Styled, div, px, rgb, ScrollHandle,
};

use crate::color::blend_rgb;
use crate::components::DropZone;
use crate::geometry::scroll_rect_in_viewport;
use crate::requests::DragOverlay;
use crate::surface::RenderData;
use crate::tree_query::{drop_target_node, first_panel_in_subtree};

use super::{main_areas::ScrollOverflow, rect_element};

pub(crate) fn render(
    data: &RenderData<'_>,
    overlay: Option<&DragOverlay>,
    overflows: &[ScrollOverflow],
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
) -> Vec<AnyElement> {
    let Some(overlay) = overlay else {
        return Vec::new();
    };
    let theme = data.theme;
    let mut elements = Vec::new();

    // Dim the source before painting drop targets so target feedback remains prominent.
    if let Some(source_rect) = drag_source_rect(data, overlay.preview.payload, overlay.handle)
        && let Some(source_rect) = visible_source_rect(
            data,
            overlay.preview.payload,
            source_rect,
            overflows,
            scroll_handles,
        )
    {
        elements.push(
            rect_element(
                source_rect,
                blend_rgb(theme.colors.window_background, theme.colors.group_background, 0.18),
            )
            .into_any_element(),
        );
    }

    for zone in &overlay.zones {
        let mut zone_rect = zone.rect;
        if let Some(target_node) = drop_target_node(zone.target)
            && let Some(overflow) = overflows.iter().find(|entry| entry.nodes.contains(&target_node))
        {
            let scroll_offset_y = scroll_handles
                .get(&overflow.owner)
                .map_or(0.0, |handle| handle.offset().y.as_f32());
            let Some(scrolled_rect) =
                scroll_rect_in_viewport(zone.rect, overflow.viewport, scroll_offset_y)
            else {
                continue;
            };
            zone_rect = scrolled_rect;
        }

        elements.push(
            DropZone {
                rect: zone_rect,
                active: overlay.preview.target == Some(zone.target),
                theme: *theme,
            }
            .into_any_element(),
        );
    }

    let label = drag_label(data, overlay.preview.payload);
    let ghost_width = theme
        .metrics
        .default_column_width
        .min(data.rects.viewport.map_or(theme.metrics.default_column_width, |rect| rect.size.width));
    let ghost_height = theme.metrics.column_header_height.min(
        data.rects.viewport.map_or(theme.metrics.column_header_height, |rect| rect.size.height),
    );
    let ghost = Rect::new(
        Point::new(
            (overlay.preview.point.x - ghost_width / 2.0).max(0.0),
            (overlay.preview.point.y - ghost_height / 2.0).max(0.0),
        ),
        Size::new(ghost_width, ghost_height),
    );
    elements.push(
        div()
            .absolute()
            .left(px(ghost.origin.x))
            .top(px(ghost.origin.y))
            .w(px(ghost.size.width))
            .h(px(ghost.size.height))
            .bg(rgb(blend_rgb(
                theme.colors.active_tab,
                theme.colors.group_background,
                0.6,
            )))
            .border(px(theme.metrics.drop_outline_thickness))
            .border_color(rgb(theme.colors.focus))
            .text_color(rgb(theme.colors.primary_text))
            .child(label)
            .into_any_element(),
    );
    elements
}

fn drag_source_rect(
    data: &RenderData<'_>,
    payload: DragPayload,
    handle: DragHandle,
) -> Option<Rect> {
    match payload {
        DragPayload::Panel { panel } if handle == DragHandle::FlyoutPanel => data
            .rects
            .panels
            .iter()
            .find(|panel_rect| panel_rect.panel == panel)
            .map(|panel_rect| panel_rect.rect)
            .or_else(|| {
                data.rects
                    .tab_headers
                    .iter()
                    .find(|header| header.panel == panel)
                    .map(|header| header.rect)
            }),
        DragPayload::Panel { panel } => data
            .rects
            .tab_headers
            .iter()
            .find(|header| header.panel == panel)
            .map(|header| header.rect)
            .or_else(|| {
                data.rects
                    .panels
                    .iter()
                    .find(|panel_rect| panel_rect.panel == panel)
                    .map(|panel_rect| panel_rect.rect)
            }),
        DragPayload::Group { group } => data
            .rects
            .groups
            .iter()
            .find(|group_rect| group_rect.group == group)
            .map(|group_rect| group_rect.rect),
        DragPayload::FloatingGroup { group } => data
            .rects
            .floating_groups
            .iter()
            .find(|floating| floating.group == group)
            .map(|floating| floating.rect),
    }
}

fn visible_source_rect(
    data: &RenderData<'_>,
    payload: DragPayload,
    rect: Rect,
    overflows: &[ScrollOverflow],
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
) -> Option<Rect> {
    let source_node = match payload {
        DragPayload::Panel { panel } => data
            .frame
            .and_then(|frame| frame.panel_node(panel))
            .or_else(|| {
                data.layout.nodes().find_map(|(id, node)| match node {
                    Node::Panel(candidate) if candidate.panel_id == panel => Some(id),
                    _ => None,
                })
            }),
        DragPayload::Group { group } => Some(group),
        DragPayload::FloatingGroup { group } => data
            .layout
            .floating_group(group)
            .and_then(|floating| data.layout.dock_area(floating.area))
            .map(|area| area.root),
    };

    let Some(source_node) = source_node else {
        return Some(rect);
    };
    let Some(overflow) = overflows
        .iter()
        .find(|entry| entry.nodes.contains(&source_node))
    else {
        return Some(rect);
    };
    let scroll_offset_y = scroll_handles
        .get(&overflow.owner)
        .map_or(0.0, |handle| handle.offset().y.as_f32());
    scroll_rect_in_viewport(rect, overflow.viewport, scroll_offset_y)
}

fn drag_label(data: &RenderData<'_>, payload: DragPayload) -> String {
    let panel_title = |panel| {
        data.layout
            .panel_instance(panel)
            .map(crate::panels::panel_title)
    };
    match payload {
        DragPayload::Panel { panel } => {
            panel_title(panel).unwrap_or_else(|| "Panel".to_owned())
        }
        DragPayload::Group { group } => first_panel_in_subtree(data.layout, group)
            .and_then(panel_title)
            .map(|title| format!("Group · {title}"))
            .unwrap_or_else(|| "Group".to_owned()),
        DragPayload::FloatingGroup { group } => data
            .layout
            .floating_group(group)
            .and_then(|floating| data.layout.dock_area(floating.area))
            .and_then(|area| first_panel_in_subtree(data.layout, area.root))
            .and_then(panel_title)
            .unwrap_or_else(|| "Floating group".to_owned()),
    }
}

