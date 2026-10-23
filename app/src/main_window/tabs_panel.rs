use gpui::{Div, Styled, div, px, rgb};
use ui::Theme;

/// Basic tab-group panel surface. Tab headers and interaction behavior will be added here.
pub(crate) struct TabsPanel {
    theme: Theme,
}

impl TabsPanel {
    pub(crate) fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub(crate) fn render(&self, width: f32, height: f32) -> Div {
        div()
            .w(px(width.max(1.0)))
            .h(px(height.max(1.0)))
            .bg(rgb(self.theme.colors.dock_background))
            .border_1()
            .border_color(rgb(self.theme.colors.border))
    }
}
