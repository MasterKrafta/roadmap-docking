use dock_core::Rect;
use gpui::{
    App, InteractiveElement, IntoElement, ParentElement, RenderOnce, StatefulInteractiveElement,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{ClickHandler, with_focus_ring};

/// A small absolutely positioned button that displays a single glyph.
#[derive(IntoElement)]
pub struct IconButton {
    id: String,
    rect: Rect,
    glyph: String,
    theme: Theme,
    border: bool,
    background_color: u32,
    hover_color: u32,
    border_color: u32,
    on_click: ClickHandler,
}

impl IconButton {
    /// Creates an icon button using the shared dock theme and click handler.
    pub fn new(
        id: impl Into<String>,
        rect: Rect,
        glyph: impl Into<String>,
        theme: &Theme,
        on_click: ClickHandler,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            glyph: glyph.into(),
            theme: *theme,
            border: false,
            background_color: theme.colors.icon_button_background,
            hover_color: theme.colors.icon_button_hover,
            border_color: theme.colors.icon_button_border,
            on_click,
        }
    }

    /// Adds the standard one-pixel border used by the group menu control.
    pub fn with_border(mut self) -> Self {
        self.border = true;
        self
    }

    /// Overrides the component background and hover colors for specialized icon buttons.
    pub fn with_colors(mut self, background_color: u32, hover_color: u32) -> Self {
        self.background_color = background_color;
        self.hover_color = hover_color;
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let hover_color = self.hover_color;
        let background_color = self.background_color;
        let border_color = self.border_color;
        let typography = self.theme.typography;
        let mut button = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(background_color))
            .text_color(rgb(self.theme.colors.primary_text))
            .font_family(typography.family)
            .text_size(px(typography.body_size))
            .line_height(px(typography.line_height))
            .hover(move |style| style.bg(rgb(hover_color)))
            .child(self.glyph);

        if self.border {
            button = button.border_1().border_color(rgb(border_color));
        }

        with_focus_ring(button.id(self.id), &self.theme).on_click(self.on_click)
    }
}
