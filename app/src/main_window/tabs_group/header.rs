use super::{close_button, collapse_button};
use gpui::{Div, InteractiveElement, ParentElement, Styled, WindowControlArea, div, px, rgb};

/// Header with native window dragging in the empty area and controls on the right.
pub(super) fn render(
    theme: ui::Theme,
    width: f32,
    height: f32,
    draggable: bool,
) -> Div {
    let drag_area = div().flex_1().h(px(height));
    let drag_area = if draggable {
        drag_area.window_control_area(WindowControlArea::Drag)
    } else {
        drag_area
    };

    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(drag_area)
        .child(collapse_button::render(theme, height))
        .child(close_button::render(theme, height))
}
