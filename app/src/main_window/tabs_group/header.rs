mod collapse_button;
mod close_button;

use gpui::{Div, ParentElement, Styled, div, px, rgb};

/// Empty header area with the collapse and close controls aligned to the right.
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
        .child(collapse_button::render(theme, height))
        .child(close_button::render(theme, height))
}
