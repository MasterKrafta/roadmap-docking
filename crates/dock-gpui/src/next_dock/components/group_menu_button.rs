//! Hamburger menu button for the next dock implementation.

use dock_core::Rect;
use gpui::{
    App, InteractiveElement, IntoElement, ParentElement, RenderOnce, StatefulInteractiveElement,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{ClickHandler, with_focus_ring};

#[derive(IntoElement)]
pub(super) struct GroupMenuButton {
    pub(super) id: String,
    pub(super) rect: Rect,
    pub(super) theme: Theme,
    pub(super) on_click: ClickHandler,
}

impl RenderOnce for GroupMenuButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let typography = theme.typography;
        let background = theme.colors.group_menu_button;
        let hover_background = theme.colors.group_menu_button_hover;

        let button = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(background))
            .text_color(rgb(theme.colors.primary_text))
            .font_family(typography.family)
            .text_size(px(typography.body_size))
            .line_height(px(typography.line_height))
            .border_1()
            .border_color(rgb(theme.colors.icon_button_border))
            .hover(move |style| style.bg(rgb(hover_background)))
            .child("☰");

        with_focus_ring(button.id(self.id), &theme).on_click(self.on_click)
    }
}
