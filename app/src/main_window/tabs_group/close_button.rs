use gpui::{Div, ParentElement, Styled, div, px, rgb};

/// Button that closes the group (behavior will be wired later).
pub(super) fn render(theme: ui::Theme, height: f32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(theme.metrics.group_control_button_width.max(1.0)))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .text_color(rgb(theme.colors.primary_text))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child("x")
}
