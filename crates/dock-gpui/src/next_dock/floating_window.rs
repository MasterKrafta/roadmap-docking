//! Native floating surface for one composition owned by NextDockWindow.

use std::{rc::Rc, time::Duration};

use gpui::{Context, IntoElement, MouseDownEvent, Render, Window, div};
use platform::windows::{self, NativeWindowHandle};

use super::view;
use super::window::NextDockWindow;

#[derive(Clone, Copy)]
struct NativeMoveDrag {
    handle: NativeWindowHandle,
    offset_x: i32,
    offset_y: i32,
}

pub(super) struct FloatingCompositionWindow {
    owner: gpui::WeakEntity<NextDockWindow>,
    composition_id: Option<u64>,
    drag: Option<NativeMoveDrag>,
    owner_subscription: Option<Subscription>,
}

impl FloatingCompositionWindow {
    pub(super) fn new(
        owner: gpui::WeakEntity<NextDockWindow>,
        composition_id: Option<u64>,
    ) -> Self {
        Self {
            owner,
            composition_id,
            drag: None,
            owner_subscription: None,
        }
    }

    pub(super) fn set_composition_id(
        &mut self,
        composition_id: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        self.composition_id = composition_id;
        self.drag = None;
        cx.notify();
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
                if let Some(composition_id) = self.composition_id {
                    owner.update(cx, |main, cx| {
                        main.finish_native_floating_drag(composition_id, cursor, cx);
                    });
                }
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


}

impl Render for FloatingCompositionWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.owner_subscription.is_none()
            && let Some(owner) = self.owner.upgrade()
        {
            self.owner_subscription =
                Some(cx.observe(&owner, |_this, _owner, cx| cx.notify()));
        }

        let Some(composition_id) = self.composition_id else {
            return div().size_full();
        };
        let Some(owner_entity) = self.owner.upgrade() else {
            window.remove_window();
            return div().size_full();
        };
        let Some((floating, layout, theme)) =
            owner_entity.read(cx).floating_composition_snapshot(composition_id)
        else {
            window.remove_window();
            return div().size_full();
        };

        let viewport = window.viewport_size();
        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);
        let owner = self.owner.clone();
        let floating_view = cx.entity().downgrade();
        let native_move_handler: view::NativeWindowMoveHandler = Rc::new(
            move |_event: &MouseDownEvent, window, app| {
                if let Some(floating_view) = floating_view.upgrade() {
                    floating_view.update(app, |this, cx| this.begin_move(window, cx));
                }
            },
        );

        view::render_floating_layout(
            &layout,
            floating.composition_index,
            width,
            height,
            window,
            &theme,
            owner,
            native_move_handler,
        )
    }
}
