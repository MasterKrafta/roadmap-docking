use gpui::{IntoElement, Styled, div, rgb};
use ui::Theme;

/// Flexible empty drag surface for the whole Group.
pub struct GroupDragArea;

impl GroupDragArea {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .min_w_0()
            .h_full()
            .cursor_move()
            .bg(rgb(theme.colors.group_grip_background))
    }
}
