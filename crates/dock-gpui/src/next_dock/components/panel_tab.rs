//! A single tab in the next dock implementation.

use dock_core::Rect;
use gpui::{
    App, InteractiveElement, IntoElement, ParentElement, RenderOnce, StatefulInteractiveElement,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{ClickHandler, with_focus_ring};

#[derive(IntoElement)]
pub(super) struct PanelTab {
    pub(super) id: String,
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) active: bool,
    pub(super) theme: Theme,
    pub(super) on_activate: ClickHandler,
}

impl RenderOnce for PanelTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let background = if self.active {
            self.theme.colors.panel_tab_active
        } else {
            self.theme.colors.panel_tab_inactive
        };

        let hover_background = if self.active {
            background
        } else {
            self.theme.colors.panel_tab_hover
        };

        let typography = self.theme.typography;
        let theme = self.theme;

        let tab = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .flex()
            .items_center()
            .justify_start()
            .px(px(8.0))
            .whitespace_nowrap()
            .overflow_hidden()
            .bg(rgb(background))
            .text_color(rgb(theme.colors.primary_text))
            .font_family(typography.family)
            .text_size(px(typography.tab_size))
            .line_height(px(typography.line_height))
            .child(self.label);

        with_focus_ring(tab.id(self.id), &theme)
            .hover(move |style| style.bg(rgb(hover_background)))
            .on_click(self.on_activate)
    }
}
