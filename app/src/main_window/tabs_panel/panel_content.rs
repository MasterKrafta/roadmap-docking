use gpui::{Div, Styled, div, px, rgb};
use ui::Theme;

/// The content surface placed directly below the tab strip.
pub(crate) struct PanelContent {
    theme: Theme,
}

impl PanelContent {
    pub(crate) fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub(crate) fn render(&self, width: f32, height: f32) -> Div {
        let colors = self.theme.colors;

        div()
            .w(px(width.max(1.0)))
            .h(px(height.max(0.0)))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border))
    }
}
