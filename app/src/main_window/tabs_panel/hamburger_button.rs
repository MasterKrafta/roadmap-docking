use gpui::{Div, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// The button shown at the right end of a tab strip.
pub(crate) struct HamburgerButton {
    theme: Theme,
}

impl HamburgerButton {
    pub(crate) fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub(crate) fn render(&self, width: f32, height: f32) -> Div {
        let colors = self.theme.colors;

        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(width.max(1.0)))
            .h(px(height.max(1.0)))
            .bg(rgb(colors.toolbar_background))
            .text_color(rgb(colors.primary_text))
            .border_1()
            .border_color(rgb(colors.border))
            .child("☰")
    }
}
