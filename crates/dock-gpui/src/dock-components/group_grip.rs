use gpui::{IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Fixed-width drag handle for the Group.
pub struct GroupGrip;

impl GroupGrip {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .w(px(32.0))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_move()
            .bg(rgb(theme.colors.group_grip_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .child("⠿")
    }
}
