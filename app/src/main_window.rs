use gpui::{Context, IntoElement, Render, Styled, Window, div, px, rgb};
use ui::Theme;

/// Minimal application window. Docking behavior will be added back only as requested.
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
        let width = size.width.as_f32().max(1.0);
        let height = size.height.as_f32().max(1.0);

        div()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background))
    }
}
