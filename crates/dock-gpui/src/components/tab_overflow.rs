use dock_core::{Point, TabOverflowRect};
use gpui::{
    IntoElement, ParentElement, Styled, div, px, rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;

use super::context_menu::{ContextMenuTarget, GroupMenuToggle};

/// Overflow affordance. Hidden tab choices are presented by the shared ContextMenu.
pub struct TabOverflow;

/// Kept as a local alias for callers that pass the shared context-menu callback.
pub(crate) type TabOverflowToggle = GroupMenuToggle;

impl TabOverflow {
    pub(crate) fn render(
        strip_rect: dock_core::Rect,
        overflow: TabOverflowRect,
        toggle_menu: TabOverflowToggle,
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
            .on_click(move |event, _, app| {
                (toggle_menu)(
                    ContextMenuTarget::Overflow(overflow.group),
                    Point::new(
                        event.position().x.as_f32(),
                        event.position().y.as_f32(),
                    ),
                    app,
                );
                app.stop_propagation();
            })
    }
}
