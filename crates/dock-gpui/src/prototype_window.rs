use gpui::{Context, Render, Window, div, prelude::*, px, rgb};
use ui::Theme;

/// Minimal R0.3 test view. This is intentionally not the static docking UI (R5).
pub struct PrototypeWindow {
    theme: Theme,
}

impl PrototypeWindow {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl Render for PrototypeWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let typography = self.theme.typography;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(colors.window_background))
            .text_color(rgb(colors.primary_text))
            .text_size(px(typography.body_size))
            .child(
                div()
                    .h(px(metrics.column_header_height))
                    .w_full()
                    .flex()
                    .items_center()
                    .px(px(metrics.content_padding))
                    .bg(rgb(colors.toolbar_background))
                    .child("Docking System — GPUI bootstrap"),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(colors.canvas_background))
                    .child("R0.3 prototype: verify window, input capture and DPI on Windows 10"),
            )
    }
}
