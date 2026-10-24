use dock_core::{DragHandle, DragPayload, Point, Rect, Size};
use gpui::{AnyElement, IntoElement, ParentElement, Styled, rgb};

use crate::components::{FloatingTitle, ResizeEdgeHandle, ResizeEdgeKind};
use crate::host::DockHost;
use crate::requests::ResizeHandle;
use crate::surface::RenderData;
use crate::tree_query::first_panel_in_subtree;

use super::{rect_element, tree::{self, TreeArea}};

pub(crate) fn render(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let rects = data.rects;
    let theme = data.theme;
    let mut elements = Vec::new();

    // Keep each window and its resize handles adjacent in paint order. Handles
    // for a lower window must not be painted over an upper overlapping window.
    for (floating_id, floating) in layout.floating_groups_in_paint_order() {
        let Some(bounds) = rects
            .floating_groups
            .iter()
            .find(|candidate| candidate.group == floating_id)
            .map(|candidate| candidate.rect)
        else {
            continue;
        };
        let Some(area) = layout.dock_area(floating.area) else {
            continue;
        };

        let nodes = tree::nodes_for_root(layout, area.root, data.frame);
        let mut container = rect_element(bounds, theme.colors.dock_background)
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

        let title = first_panel_in_subtree(layout, area.root)
            .and_then(|panel| layout.panel_instance(panel))
            .map(|instance| host.panel_title(instance).to_string())
            .unwrap_or_else(|| "Floating group".to_owned());
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
                on_close: None,
                on_collapse: None,
            }
            .into_any_element(),
        );
        elements.push(container.into_any_element());

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
                }
                .into_any_element(),
            );
        }
    }

    elements
}
