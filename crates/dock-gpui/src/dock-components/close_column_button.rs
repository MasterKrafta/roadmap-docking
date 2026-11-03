use gpui::{IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Visual close control for the whole Column.
pub struct CloseColumnButton;

impl CloseColumnButton {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .w(px(32.0))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(theme.colors.icon_button_background))
            .border_1()
            .border_color(rgb(theme.colors.icon_button_border))
            .text_color(rgb(theme.colors.primary_text))
            .child("×")
    }
}
