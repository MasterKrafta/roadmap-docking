use dock_core::Rect;
use gpui::{App, IntoElement, RenderOnce, Styled, Window, div, px, rgb};
use ui::Theme;

/// The outer frame around a dock group, including its shared background and border.
#[derive(IntoElement)]
pub struct GroupFrame {
    /// Absolute group bounds supplied by Core's frame geometry.
    pub rect: Rect,
    /// Shared application theme.
    pub theme: Theme,
}

impl RenderOnce for GroupFrame {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.group_frame_background))
            .border_1()
            .border_color(rgb(self.theme.colors.group_frame_border))
    }
}
