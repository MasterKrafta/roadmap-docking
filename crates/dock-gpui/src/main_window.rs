use dock_core::{DockCommand, DragHandle, DragPayload, Point, Rect, Size};
use gpui::{
    Context, DispatchPhase, IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement, Render, Styled, Subscription, Window, div, px, rgb,
};
use ui::Theme;

use crate::dock::{DockController, WindowHost};
use crate::requests::ResizeHandle;
use crate::DockSurface;

/// Host window for the dock surface. Commands and pointer gestures dispatch through
/// the controller; menu state remains for a later roadmap stage.
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

    pub(crate) fn begin_drag(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        position: gpui::Point<gpui::Pixels>,
    ) {
        let pointer = dock_surface_pointer(
            position,
            self.theme.metrics.column_header_height.max(1.0),
        );
        if let Err(error) = self.dock.begin_drag(payload, handle, pointer) {
            tracing::debug!(error = ?error, "could not record dock drag pointer-down");
        }
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dock.resize_session().is_some() {
            let pointer = dock_pointer(event.position);
            if event.pressed_button != Some(MouseButton::Left) {
                // A move without the left button means the release event was lost.
                // Finish the gesture here so the resize session cannot remain stuck.
                if let Err(error) = self.dock.commit_resize(pointer) {
                    tracing::debug!(error = ?error, "dock resize commit after lost mouse-up was rejected");
                }
                cx.notify();
                return;
            }

            if let Err(error) = self.dock.update_resize(pointer) {
                tracing::debug!(error = ?error, "dock resize preview was rejected");
            }

            cx.notify();
            return;
        }

        let pointer = dock_surface_pointer(
            event.position,
            self.theme.metrics.column_header_height.max(1.0),
        );
        if event.pressed_button != Some(MouseButton::Left) {
            // The release event may have been lost while a drag was active.
            if self.dock.cancel_drag() {
                cx.notify();
            }
            return;
        }

        let viewport = self.dock_viewport(window);
        let frame = if self.dock.drag_needs_start(pointer) {
            match self.dock.frame(viewport) {
                Ok(frame) => Some(frame),
                Err(error) => {
                    tracing::warn!(error = ?error, "could not compute geometry to start dock drag");
                    if self.dock.cancel_drag() {
                        cx.notify();
                    }
                    return;
                }
            }
        } else {
            None
        };

        match self.dock.update_drag(pointer, viewport, frame.as_ref()) {
            Ok(changed) => {
                if changed {
                    cx.notify();
                }
            }
            Err(error) => {
                tracing::warn!(error = ?error, "dock drag pointer update failed");
                self.dock.cancel_drag();
                // update_drag may already have cancelled a stale session internally.
                // Always redraw so a removed overlay cannot remain on screen.
                cx.notify();
            }
        }
    }

    pub(crate) fn handle_surface_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        if self.dock.resize_session().is_none() {
            let had_overlay = self.dock.drag_overlay().is_some();
            let pointer = dock_surface_pointer(
                event.position,
                self.theme.metrics.column_header_height.max(1.0),
            );
            let mut changed = had_overlay;
            let viewport = self.dock_viewport(window);
            match self.dock.finish_drag(pointer, viewport) {
                Ok(Some(request)) => {
                    changed = true;
                    match self.dock.commit_drop_request(request, viewport) {
                        Ok(action) => {
                            tracing::debug!(action = ?action, "dock drag request resolved and committed");
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "dock drag request was rejected");
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    changed = true;
                    tracing::warn!(error = ?error, "could not finish dock drag");
                }
            }
            // Also clears the active overlay when a drag ends without a valid target.
            if changed {
                cx.notify();
            }
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

/// Converts window coordinates to the DockSurface's local geometry coordinates.
fn dock_surface_pointer(position: gpui::Point<gpui::Pixels>, surface_top: f32) -> Point {
    let pointer = dock_pointer(position);
    Point::new(pointer.x, pointer.y - surface_top)
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
                    if !window.is_window_active()
                        && (this.dock.cancel_resize() || this.dock.cancel_drag())
                    {
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

        // Escape cancels an active pointer gesture before it can leave stale state.
        let escape_key = cx.listener::<KeyDownEvent>(|this, event, _window, cx| {
            if event.keystroke.key.eq_ignore_ascii_case("escape")
                && (this.dock.cancel_resize() || this.dock.cancel_drag())
            {
                cx.stop_propagation();
                cx.notify();
            }
        });
        window.on_key_event(
            move |event: &KeyDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && event.keystroke.key.eq_ignore_ascii_case("escape")
                {
                    escape_key(event, window, cx);
                }
            },
        );

        // Element mouse-move handlers stop receiving events once the pointer leaves
        // the surface hitbox. Keep tracking active pointer gestures at window scope
        // while the pointer is outside the surface.
        let outside_move = cx.listener::<MouseMoveEvent>(|this, event, window, cx| {
            this.handle_surface_mouse_move(event, window, cx);
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

        let outside_up = cx.listener::<MouseUpEvent>(|this, event, window, cx| {
            this.handle_surface_mouse_up(event, window, cx);
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
                    self.dock.drag_overlay(),
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

