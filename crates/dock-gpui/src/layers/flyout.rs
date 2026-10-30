use dock_core::{DockCommand, DragHandle, DragPayload, Point, Rect, ResizeEdge, Size};
use gpui::{AnyElement, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, rgb};

use crate::components::{FlyoutHeader, ResizeEdgeHandle, ResizeEdgeKind};
use crate::host::DockHost;
use crate::requests::{DockMenuRequest, ResizeHandle};
use crate::surface::RenderData;

use super::rect_element;

pub(crate) fn render(data: &RenderData<'_>, host: &mut dyn DockHost) -> Vec<AnyElement> {
    let layout = data.layout;
    let rects = data.rects;
    let theme = data.theme;
    let mut elements = Vec::new();

    if let Some(flyout) = rects.flyout
        && let Some(instance) = layout.panel_instance(flyout.panel)
    {
        let header_height = theme
            .metrics
            .column_header_height
            .min(flyout.rect.size.height);
        let header = FlyoutHeader {
            group: flyout.group,
            panel: flyout.panel,
            rect: Rect::new(
                Point::ZERO,
                Size::new(flyout.rect.size.width, header_height),
            ),
            title: host.panel_title(instance).to_string(),
            theme: *theme,
            on_menu: host.menu(DockMenuRequest::PanelGroup {
                group: flyout.group,
                panel: Some(flyout.panel),
            }),
            on_close: host.command(DockCommand::CloseFlyout),
        };

        let content_rect = Rect::new(
            Point::new(0.0, header_height),
            Size::new(
                flyout.rect.size.width,
                (flyout.rect.size.height - header_height).max(0.0),
            ),
        );
        let content = super::rect_element(content_rect, theme.colors.group_background)
            .child(host.render_panel(instance))
            .id(format!("dock-flyout-panel-{:?}", flyout.panel))
            .on_mouse_down(
                MouseButton::Left,
                host.drag_start(
                    DragPayload::Panel {
                        panel: flyout.panel,
                    },
                    DragHandle::FlyoutPanel,
                ),
            );

        let overlay_element = rect_element(flyout.rect, theme.colors.dock_background)
            .border_1()
            .border_color(rgb(theme.colors.border))
            .child(header.into_any_element())
            .child(content.into_any_element());
        elements.push(overlay_element.into_any_element());

        let edge_width = theme.metrics.splitter_hit_zone.min(flyout.rect.size.width);
        let edge_x = match flyout.resize_edge {
            ResizeEdge::Left => Some(flyout.rect.origin.x - edge_width / 2.0),
            ResizeEdge::Right => {
                Some(flyout.rect.origin.x + flyout.rect.size.width - edge_width / 2.0)
            }
            _ => None,
        };
        if edge_width > 0.0
            && flyout.rect.size.height > 0.0
            && let Some(edge_x) = edge_x
        {
            elements.push(
                ResizeEdgeHandle {
                    id: format!("dock-flyout-resize-{:?}", flyout.panel),
                    rect: Rect::new(
                        Point::new(edge_x, flyout.rect.origin.y),
                        Size::new(edge_width, flyout.rect.size.height),
                    ),
                    edge: ResizeEdgeKind::Flyout,
                    theme: *theme,
                    on_resize_start: host.resize_start(ResizeHandle::Flyout {
                        panel: flyout.panel,
                    }),
                    on_resize_key: Some(host.resize_key(ResizeHandle::Flyout {
                        panel: flyout.panel,
                    })),
                }
                .into_any_element(),
            );
        }
    }

    elements
}
