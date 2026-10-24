use dock_core::Rect;
use gpui::{App, IntoElement, RenderOnce, Styled, Window, div, px, rgb};
use ui::Theme;

use crate::color::blend_rgb;

/// A drop target highlight with distinct active and idle presentation.
#[derive(IntoElement)]
pub struct DropZone {
    /// Absolute bounds for this drop target.
    pub rect: Rect,
    /// Whether the current drag preview targets this zone.
    pub active: bool,
    /// Shared application theme.
    pub theme: Theme,
}

impl RenderOnce for DropZone {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let weight = if self.active { 0.38 } else { 0.13 };
        let fill = blend_rgb(
            self.theme.colors.drop_target,
            self.theme.colors.canvas_background,
            weight,
        );
        let border = if self.active {
            self.theme.colors.focus
        } else {
            self.theme.colors.drop_target
        };

        div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(fill))
            .border_1()
            .border_color(rgb(border))
    }
}
