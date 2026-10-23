use gpui::{Context, IntoElement, Render, Styled, Window, div, px, rgb};
use ui::Theme;

/// Minimal application window.
///
/// Dock panels, columns, floating windows, and pointer gestures will be rebuilt
/// from this small root instead of carrying the previous UI implementation forward.
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
        let viewport = window.viewport_size();

        div()
            .w(px(viewport.width.as_f32().max(1.0)))
            .h(px(viewport.height.as_f32().max(1.0)))
            .bg(rgb(self.theme.colors.canvas_background))
    }
}
