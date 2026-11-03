use gpui::{IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Fixed-width visual group menu control.
pub struct GroupMenuButton;

impl GroupMenuButton {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .w(px(32.0))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(theme.colors.group_menu_button))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .child("☰")
    }
}
