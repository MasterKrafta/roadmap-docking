use gpui::{div, prelude::*, px, rgb, Context, Entity, MouseButton, Window};

use crate::DockState;

/// Верхняя полоска хедера floating-окна, занятая нативной resize-рамкой
/// (SM_CYSIZEFRAME + SM_CXPADDEDBORDER ≈ 8 px при 100% масштаба).
const RESIZE_ZONE: f32 = 8.0;

pub(crate) struct PanelView {
    pub(crate) dock: Entity<DockState>,
}

impl Render for PanelView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let dock = self.dock.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .child(
                div()
                    .id("panel-header")
                    .h(px(28.0))
                    .w_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(rgb(0x3c3c3c))
                    .text_color(rgb(0xdddddd))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                        // Пока floating-окно на экране, докнутая копия панели не
                        // рисуется, значит клик пришёл из floating. Верхняя рамка
                        // у него — это resize, а не drag.
                        if dock.read(cx).floating_visible && event.position.y < px(RESIZE_ZONE) {
                            return;
                        }

                        dock.update(cx, |dock, cx| dock.begin_drag(cx));
                    })
                    .child("Brushes — тяни за заголовок"),
            )
            .child(
                div()
                    .p_3()
                    .text_color(rgb(0xaaaaaa))
                    .child("Содержимое панели"),
            )
    }
}
