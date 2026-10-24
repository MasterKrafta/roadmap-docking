use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};

/// Minimal application window used as the clean starting point for the docking UI.
pub(crate) struct MainWindow {
    theme: ui::Theme,
}

impl MainWindow {
    pub(crate) fn new(theme: ui::Theme) -> Result<Self, String> {
        Ok(Self { theme })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(viewport.width.as_f32().max(1.0)))
            .h(px(viewport.height.as_f32().max(1.0)))
            .bg(rgb(self.theme.colors.window_background))
            .text_color(rgb(self.theme.colors.primary_text))
            .child("Docking window")
    }
}
