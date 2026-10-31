use std::time::Duration;

use dock_core::{DockCommand, FlyoutRect, ResizeEdge};
use gpui::{
    Animation, AnimationExt, AnyView, App, Entity, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::DockController;

/// Interactive flyout surface rendered above the Canvas without changing its layout.
///
/// Geometry and resize constraints come from dock-core. The adapter owns only
/// pointer gestures and presentation details; moving the panel is resolved by
/// the shared Core drag/drop pipeline.
pub struct FlyoutView;

impl FlyoutView {
    pub(crate) fn render(
        flyout: FlyoutRect,
        title: SharedString,
        panel_view: Option<AnyView>,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let rect = flyout.rect;
        let header_height = theme.metrics.column_header_height.min(rect.size.height);
        let close_size = theme.metrics.group_control_button_width.min(header_height);
        let resize_direction = resize_width_direction(flyout.resize_edge);

        let drag_grip = div()
            .id(format!("flyout-drag-{:?}", flyout.panel))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .px(px(10.0))
            .overflow_hidden()
            .cursor_move()
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            );

        let close_button = div()
            .id(format!("flyout-close-{:?}", flyout.panel))
            .w(px(close_size))
            .h(px(close_size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.icon_button_background))
            .border_1()
            .border_color(rgb(theme.colors.icon_button_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(18.0))
            .child("×")
            .on_click({
                let controller = controller.clone();
                move |_, _, app| {
                    dispatch_transient(&controller, DockCommand::CloseFlyout, app);
                }
            });

        let header = div()
            .w_full()
            .h(px(header_height))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme.metrics.button_gap))
            .pr(px(4.0))
            .bg(rgb(theme.colors.flyout_header_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(drag_grip)
            .child(close_button);

        let mut body = div()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .bg(rgb(theme.colors.panel_surface_background));
        if let Some(panel_view) = panel_view {
            body = body.child(div().size_full().child(panel_view));
        }

        let mut surface = div()
            .id(format!("dock-flyout-{:?}", flyout.panel))
            .absolute()
            .left(px(rect.origin.x))
            .top(px(rect.origin.y))
            .w(px(rect.size.width))
            .h(px(rect.size.height))
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme.colors.panel_surface_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .shadow_lg()
            .child(header)
            .child(body);

        if resize_direction.is_some() {
            let hit_width = 7.0_f32.min(rect.size.width.max(0.0));
            let edge_left = match flyout.resize_edge {
                ResizeEdge::Right => (rect.size.width - hit_width).max(0.0),
                ResizeEdge::Left => 0.0,
                _ => 0.0,
            };
            let line_left = match flyout.resize_edge {
                ResizeEdge::Right => (hit_width - 2.0).max(0.0),
                ResizeEdge::Left => 0.0,
                _ => 0.0,
            };

            let resize_edge = div()
                .id(format!("flyout-resize-{:?}", flyout.panel))
                .absolute()
                .left(px(edge_left))
                .top_0()
                .w(px(hit_width))
                .h_full()
                .cursor_col_resize()
                .child(
                    div()
                        .absolute()
                        .left(px(line_left))
                        .top_0()
                        .w(px(2.0))
                        .h_full()
                        .bg(rgb(theme.colors.resize_edge_handle)),
                );

            surface = surface.child(resize_edge);
        }

        // GPUI's AnimationExt automatically renders the final state without
        // scheduling frames when App::reduce_motion is enabled.
        let travel = match flyout.resize_edge {
            ResizeEdge::Right => -12.0,
            ResizeEdge::Left => 12.0,
            _ => 0.0,
        };
        let origin_x = rect.origin.x;
        surface.with_animation(
            format!("flyout-enter-{:?}", flyout.panel),
            Animation::new(Duration::from_millis(180)),
            move |element, progress| element.left(px(origin_x + travel * (1.0 - progress))),
        )
    }
}

fn dispatch_transient(controller: &Entity<DockController>, command: DockCommand, app: &mut App) {
    controller.update(app, |controller, cx| {
        if let Err(error) = controller.dispatch_transient_in(command, cx) {
            tracing::debug!(%error, "flyout transient command was rejected");
        }
    });
}

fn resize_width_direction(edge: ResizeEdge) -> Option<f32> {
    match edge {
        ResizeEdge::Right => Some(1.0),
        ResizeEdge::Left => Some(-1.0),
        ResizeEdge::Top
        | ResizeEdge::Bottom
        | ResizeEdge::TopLeft
        | ResizeEdge::TopRight
        | ResizeEdge::BottomLeft
        | ResizeEdge::BottomRight => None,
    }
}

#[cfg(test)]
mod tests {
    use dock_core::ResizeEdge;

    use super::resize_width_direction;

    #[test]
    fn canvas_facing_edges_grow_the_flyout_toward_the_canvas() {
        assert_eq!(resize_width_direction(ResizeEdge::Right), Some(1.0));
        assert_eq!(resize_width_direction(ResizeEdge::Left), Some(-1.0));
    }

    #[test]
    fn non_horizontal_edges_do_not_render_a_flyout_width_handle() {
        assert_eq!(resize_width_direction(ResizeEdge::Top), None);
        assert_eq!(resize_width_direction(ResizeEdge::Bottom), None);
    }
}
