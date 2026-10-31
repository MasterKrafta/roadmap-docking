use dock_core::TabOverflowRect;
use gpui::{
    IntoElement, ParentElement, Styled, div, px, rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;


/// Overflow affordance. Hidden tab choices are presented by the shared ContextMenu.
pub struct TabOverflow;

impl TabOverflow {
    pub(crate) fn render(
        strip_rect: dock_core::Rect,
        overflow: TabOverflowRect,
        theme: Theme,
    ) -> impl IntoElement {
        let left = overflow.button_rect.origin.x - strip_rect.origin.x;
        let top = overflow.button_rect.origin.y - strip_rect.origin.y;
        let button_width = overflow.button_rect.size.width;
        let button_height = overflow.button_rect.size.height;

        div()
            .id(format!("tab-overflow-{:?}", overflow.group))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(button_width))
            .h(px(button_height))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.tab_overflow_button))
            .hover(|style| style.bg(rgb(theme.colors.tab_overflow_button_hover)))
            .text_color(rgb(theme.colors.primary_text))
            .child("»»")
    }
}
