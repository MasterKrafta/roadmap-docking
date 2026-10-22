use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use ui::Theme;

/// Minimal application window. Docking UI will be rebuilt here incrementally.
pub(crate) struct MainWindow {
    theme: Theme,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        Ok(Self { theme })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .w(px(size.width.as_f32().max(1.0)))
            .h(px(size.height.as_f32().max(1.0)))
            .bg(rgb(self.theme.colors.window_background))
            .text_color(rgb(self.theme.colors.primary_text))
            .child("Roadmap Docking")
    }
}
