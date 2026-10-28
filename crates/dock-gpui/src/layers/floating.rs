use std::collections::HashSet;

use dock_core::{
    DockAreaId, DockCommand, DragHandle, DragPayload, FloatingGroupId, Node, Point, Rect, Size,
};
use gpui::{AnyElement, IntoElement, ParentElement, Styled, rgb};

use crate::components::{FloatingTitle, ResizeEdgeHandle, ResizeEdgeKind};
use crate::host::DockHost;
use crate::labels::FLOATING_GROUP_TITLE;
use crate::requests::ResizeHandle;
use crate::surface::RenderData;
use crate::tree_query::first_panel_in_subtree;

use super::{
    rect_element,
    tree::{self, TreeArea},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FloatingGroupRenderMode {
    /// Paint the floating group over the main dock surface.
    DockSurface,
    /// Paint the group's contents inside a dedicated native window.
    NativeWindow,
}

pub(crate) fn render(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    excluded_groups: Option<&HashSet<FloatingGroupId>>,
) -> Vec<AnyElement> {
    let mut elements = Vec::new();

    // Keep each window and its resize handles adjacent in paint order. Handles
    // for a lower window must not be painted over an upper overlapping window.
    for (floating_id, floating) in data.layout.floating_groups_in_paint_order() {
        if excluded_groups.is_some_and(|excluded| excluded.contains(&floating_id)) {
            continue;
        }

        let Some(bounds) = data
            .rects
            .floating_groups
            .iter()
            .find(|candidate| candidate.group == floating_id)
            .map(|floating| floating.rect)
        else {
            continue;
        };

        elements.extend(render_group(
            data,
            host,
            floating_id,
            floating.area,
            bounds,
            FloatingGroupRenderMode::DockSurface,
        ));
    }

    elements
}

/// Renders one floating group independently from the collection of floating groups.
///
/// `bounds` are in the coordinate system of the layout frame. When rendering a
/// native window, the geometry origin remains the original frame origin so each
/// element can be translated into the child window's local coordinate system.
pub(crate) fn render_group(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    floating_id: FloatingGroupId,
    area_id: DockAreaId,
    bounds: Rect,
    mode: FloatingGroupRenderMode,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let rects = data.rects;
    let theme = data.theme;
    let mut elements = Vec::new();

    let Some(area) = layout.dock_area(area_id) else {
        return elements;
    };

    let container_bounds = match mode {
        FloatingGroupRenderMode::DockSurface => bounds,
        FloatingGroupRenderMode::NativeWindow => {
            Rect::new(Point::ZERO, bounds.size)
        }
    };
    let nodes = tree::nodes_for_root(layout, area.root, data.frame);
    let mut container = rect_element(container_bounds, theme.colors.dock_background)
        .overflow_hidden()
        .border_1()
        .border_color(rgb(theme.colors.border));

    for element in tree::elements(
        data,
        host,
        TreeArea {
            rects,
            nodes: &nodes,
            origin: bounds.origin,
            content_column: None,
        },
    ) {
        container = container.child(element);
    }

    if mode == FloatingGroupRenderMode::DockSurface {
        let title = first_panel_in_subtree(layout, area.root)
            .and_then(|panel| layout.panel_instance(panel))
            .map(|instance| host.panel_title(instance).to_string())
            .unwrap_or_else(|| FLOATING_GROUP_TITLE.to_owned());
        let collapse_target = match layout.node(area.root) {
            Some(Node::Tabs(tabs)) => Some((area.root, tabs.collapsed)),
            Some(Node::Column(column)) if column.groups.len() == 1 => {
                let group = column.groups[0];
                match layout.node(group) {
                    Some(Node::Tabs(tabs)) => Some((group, tabs.collapsed)),
                    _ => None,
                }
            }
            _ => None,
        };
        let collapse_label = collapse_target
            .map(|(_, collapsed)| if collapsed { "+" } else { "−" })
            .unwrap_or("−");
        let on_collapse = collapse_target.map(|(group, collapsed)| {
            host.command(DockCommand::CollapseGroup {
                group,
                collapsed: !collapsed,
            })
        });

        let title_height = theme.metrics.column_header_height.min(bounds.size.height);
        let title_rect = Rect::new(Point::ZERO, Size::new(bounds.size.width, title_height));
        container = container.child(
            FloatingTitle {
                group: floating_id,
                rect: title_rect,
                title,
                theme: *theme,
                on_drag_start: host.drag_start(
                    DragPayload::FloatingGroup { group: floating_id },
                    DragHandle::FloatingTitle,
                ),
                on_capture_drag_start: matches!(
                    layout.node(area.root),
                    Some(Node::Tabs(_) | Node::Panel(_))
                )
                .then(|| {
                    host.drag_start(
                        DragPayload::FloatingGroup { group: floating_id },
                        DragHandle::FloatingCapture,
                    )
                }),
                on_close: Some(host.close_floating(floating_id)),
                on_collapse,
                collapse_label,
            }
            .into_any_element(),
        );
    }

    elements.push(container.into_any_element());

    if mode == FloatingGroupRenderMode::DockSurface {
        for handle in rects
            .floating_resize_edges
            .iter()
            .rev()
            .filter(|handle| handle.group == floating_id)
        {
            elements.push(
                ResizeEdgeHandle {
                    id: format!("dock-floating-resize-{:?}", handle.group),
                    rect: handle.rect,
                    edge: ResizeEdgeKind::Floating(handle.edge),
                    theme: *theme,
                    on_resize_start: host.resize_start(ResizeHandle::FloatingEdge {
                        group: handle.group,
                        edge: handle.edge,
                    }),
                    on_resize_key: Some(host.resize_key(ResizeHandle::FloatingEdge {
                        group: handle.group,
                        edge: handle.edge,
                    })),
                }
                .into_any_element(),
            );
        }
    }

    elements
}
