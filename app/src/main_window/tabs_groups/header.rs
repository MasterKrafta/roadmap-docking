use gpui::{Div, ParentElement, Styled, div, px, rgb};

/// Empty title/drag area on the left; group controls stay aligned on the right.
pub(super) fn render(theme: ui::Theme, width: f32, height: f32) -> Div {
    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(div().flex_1().h(px(height)))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(34.0))
                .h(px(height))
                .bg(rgb(theme.colors.toolbar_background))
                .text_color(rgb(theme.colors.primary_text))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child(">>"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(34.0))
                .h(px(height))
                .bg(rgb(theme.colors.toolbar_background))
                .text_color(rgb(theme.colors.primary_text))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child("x"),
        )
}
