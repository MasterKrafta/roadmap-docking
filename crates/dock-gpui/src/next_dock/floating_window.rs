//! Native floating surface for one composition owned by NextDockWindow.

use std::time::Duration;

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent, ParentElement, Render,
    Styled, Subscription, Window, div, px, rgb,
};
use platform::windows::{self, NativeWindowHandle};

use super::view;
use super::window::{CompositionDropZone, NextDockWindow};

const HEADER_HEIGHT: f32 = 30.0;

#[derive(Clone, Copy)]
struct NativeMoveDrag {
    handle: NativeWindowHandle,
    offset_x: i32,
    offset_y: i32,
}

pub(super) struct FloatingCompositionWindow {
    owner: gpui::WeakEntity<NextDockWindow>,
    composition_id: u64,
    drag: Option<NativeMoveDrag>,
    owner_subscription: Option<Subscription>,
}

impl FloatingCompositionWindow {
    pub(super) fn new(
        owner: gpui::WeakEntity<NextDockWindow>,
        composition_id: u64,
    ) -> Self {
        Self {
            owner,
            composition_id,
            drag: None,
            owner_subscription: None,
        }
    }

    pub(super) fn begin_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_some() || !windows::left_button_down() {
            return;
        }
        let Some(handle) = NativeWindowHandle::from_window(window) else {
            return;
        };
        let Some((left, top, _, _)) = windows::window_rect(handle) else {
            return;
        };
        let cursor = windows::cursor_position();
        self.drag = Some(NativeMoveDrag {
            handle,
            offset_x: cursor.0 - left,
            offset_y: cursor.1 - top,
        });

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;

                if !matches!(this.update(cx, |view, cx| view.tick_move(cx)), Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_move(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };

        let cursor = windows::cursor_position();
        if !windows::left_button_down() {
            self.drag = None;
            if let Some(owner) = self.owner.upgrade() {
                owner.update(cx, |main, cx| {
                    main.finish_native_floating_drag(self.composition_id, cursor, cx);
                });
            }
            return false;
        }

        let _moved = windows::move_window(
            drag.handle,
            cursor.0 - drag.offset_x,
            cursor.1 - drag.offset_y,
        );

        if let Some(owner) = self.owner.upgrade() {
            owner.update(cx, |main, cx| {
                main.update_native_floating_drop_zone(cursor, cx);
            });
        }
        true
    }

    fn dock_back(&mut self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner.upgrade() {
            owner.update(cx, |main, cx| {
                main.dock_floating_composition(
                    self.composition_id,
                    CompositionDropZone::RightEdge,
                    cx,
                );
            });
        }
    }

    fn handle_surface_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if let Some(owner) = self.owner.upgrade() {
            owner.update(cx, |main, cx| main.update_resize(event, cx));
        }
    }

    fn handle_surface_mouse_up(&mut self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner.upgrade() {
            owner.update(cx, |main, cx| main.end_resize(cx));
        }
    }
}

impl Render for FloatingCompositionWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.owner_subscription.is_none()
            && let Some(owner) = self.owner.upgrade()
        {
            self.owner_subscription =
                Some(cx.observe(&owner, |_this, _owner, cx| cx.notify()));
        }

        let Some(owner_entity) = self.owner.upgrade() else {
            window.remove_window();
            return div().size_full();
        };
        let Some((floating, layout, theme)) =
            owner_entity.read(cx).floating_composition_snapshot(self.composition_id)
        else {
            window.remove_window();
            return div().size_full();
        };

        let viewport = window.viewport_size();
        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);
        let content_height = (height - HEADER_HEIGHT).max(1.0);
        let owner = self.owner.clone();

        let drag_handle = div()
            .id("floating-composition-drag-handle")
            .flex_1()
            .h(px(HEADER_HEIGHT))
            .px_2()
            .flex()
            .items_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.toolbar_background))
            .text_color(rgb(theme.colors.primary_text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| this.begin_move(window, cx)),
            )
            .child("Плавающая панель");

        let dock_button = div()
            .id("floating-composition-dock")
            .h(px(HEADER_HEIGHT))
            .px_3()
            .flex()
            .items_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.toolbar_background))
            .text_color(rgb(theme.colors.primary_text))
            .on_click(cx.listener(|this, _event, _window, cx| this.dock_back(cx)))
            .child("В док");

        let header = div()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .flex()
            .flex_row()
            .bg(rgb(theme.colors.toolbar_background))
            .border_b_1()
            .border_color(rgb(theme.colors.border))
            .child(drag_handle)
            .child(dock_button);

        let content = view::render_floating_layout(
            &layout,
            floating.composition_index,
            width,
            content_height,
            window,
            &theme,
            owner,
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme.colors.window_background))
            .child(header)
            .child(content)
            .on_mouse_move(
                cx.listener(|this, event, _window, cx| this.handle_surface_mouse_move(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| this.handle_surface_mouse_up(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| this.handle_surface_mouse_up(cx)),
            )
    }
}
