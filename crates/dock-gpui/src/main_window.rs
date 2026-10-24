use dock_core::{DockCommand, Point, Rect, Size};
use gpui::{
    Context, DispatchPhase, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement, Render, Styled, Subscription, Window, div, px, rgb,
};
use ui::Theme;

use crate::dock::{DockController, WindowHost};
use crate::requests::ResizeHandle;
use crate::DockSurface;

/// Host window for the dock surface. Commands and resize gestures dispatch through
/// the controller; menu and drag handling are connected in their later roadmap stages.
pub(crate) struct MainWindow {
    theme: Theme,
    dock: DockController,
    activation_subscription: Option<Subscription>,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        Ok(Self {
            dock: DockController::new_initial(&theme)?,
            theme,
            activation_subscription: None,
        })
    }

    pub(crate) fn apply_dock_command(&mut self, command: DockCommand) {
        let _ = self.dock.apply(command);
    }

    pub(crate) fn begin_resize(
        &mut self,
        handle: ResizeHandle,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let frame = match self.dock.frame(self.dock_viewport(window)) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(error = ?error, "could not compute frame to begin dock resize");
                return;
            }
        };

        if let Err(error) = self.dock.begin_resize(handle, dock_pointer(position), &frame) {
            tracing::warn!(error = ?error, "could not begin dock resize");
            return;
        }

        cx.notify();
    }

    pub(crate) fn handle_surface_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if self.dock.resize_session().is_none() {
            return;
        }

        let pointer = dock_pointer(event.position);
        if event.pressed_button == Some(MouseButton::Left) {
            if let Err(error) = self.dock.update_resize(pointer) {
                tracing::debug!(error = ?error, "dock resize preview was rejected");
            }
        } else if let Err(error) = self.dock.commit_resize(pointer) {
            tracing::debug!(error = ?error, "dock resize commit was rejected");
        }

        cx.notify();
    }

    pub(crate) fn handle_surface_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.dock.resize_session().is_none() {
            return;
        }

        if let Err(error) = self.dock.commit_resize(dock_pointer(event.position)) {
            tracing::debug!(error = ?error, "dock resize commit was rejected");
        }

        cx.notify();
    }

    fn dock_viewport(&self, window: &Window) -> Rect {
        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let menu_height = self.theme.metrics.column_header_height.max(1.0);
        Rect::new(
            Point::ZERO,
            Size::new(width, (height - menu_height).max(1.0)),
        )
    }
}

fn dock_pointer(position: gpui::Point<gpui::Pixels>) -> Point {
    Point::new(position.x.as_f32(), position.y.as_f32())
}

fn surface_contains_window_point(
    position: gpui::Point<gpui::Pixels>,
    width: f32,
    top: f32,
    height: f32,
) -> bool {
    let x = position.x.as_f32();
    let y = position.y.as_f32();
    x >= 0.0 && x < width && y >= top && y < top + height
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation_subscription.is_none() {
            self.activation_subscription = Some(cx.observe_window_activation(
                window,
                |this, window, cx| {
                    if !window.is_window_active() && this.dock.cancel_resize() {
                        cx.notify();
                    }
                },
            ));
        }

        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let menu_height = self.theme.metrics.column_header_height.max(1.0);
        let content_height = (height - menu_height).max(1.0);

        // Element mouse-move handlers stop receiving events once the pointer leaves
        // the surface hitbox. Keep tracking the same resize gesture at window scope
        // only while the pointer is outside the surface.
        let outside_move = cx.listener::<MouseMoveEvent>(|this, event, _window, cx| {
            this.handle_surface_mouse_move(event, cx);
        });
        window.on_mouse_event(
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && !surface_contains_window_point(
                        event.position,
                        width,
                        menu_height,
                        content_height,
                    )
                {
                    outside_move(event, window, cx);
                }
            },
        );

        let outside_up = cx.listener::<MouseUpEvent>(|this, event, _window, cx| {
            this.handle_surface_mouse_up(event, cx);
        });
        window.on_mouse_event(
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == MouseButton::Left
                    && !surface_contains_window_point(
                        event.position,
                        width,
                        menu_height,
                        content_height,
                    )
                {
                    outside_up(event, window, cx);
                }
            },
        );

        let viewport = Rect::new(Point::ZERO, Size::new(width, content_height));

        let frame = match self.dock.frame(viewport) {
            Ok(frame) => frame,
            Err(error) => {
                return div()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.window_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("Could not compute dock geometry: {error:?}"));
            }
        };
        for overflow in &frame.rects().vertical_overflow {
            self.dock
                .scroll_handles_mut()
                .entry(overflow.owner)
                .or_insert_with(gpui::ScrollHandle::new);
        }

        let mut host = WindowHost::new(cx);
        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(width))
                    .h(px(menu_height))
                    .bg(rgb(self.theme.colors.toolbar_background))
                    .border_1()
                    .border_color(rgb(self.theme.colors.border))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child("Dockers ▾"),
            )
            .child(
                DockSurface::new(
                    self.dock.layout(),
                    &frame,
                    self.dock.scroll_handles(),
                    &self.theme,
                    None,
                )
                .build(&mut host),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::surface_contains_window_point;
    use gpui::{point, px};

    #[test]
    fn surface_contains_top_left_but_excludes_right_and_bottom_edges() {
        let contains = |x, y| {
            surface_contains_window_point(point(px(x), px(y)), 200.0, 24.0, 76.0)
        };

        assert!(contains(0.0, 24.0));
        assert!(contains(199.0, 99.0));
        assert!(!contains(200.0, 30.0));
        assert!(!contains(-1.0, 30.0));
        assert!(!contains(20.0, 23.0));
        assert!(!contains(20.0, 100.0));
    }
}

