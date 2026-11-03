use gpui::{IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Visual collapse-to-icons control in the Column header.
pub struct CollapseColumnButton;

impl CollapseColumnButton {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        Self::render_with_metrics(id, theme, 32.0, 14.0)
    }

    pub fn render_with_metrics(
        id: String,
        theme: Theme,
        width: f32,
        icon_size: f32,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w(px(width))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(theme.colors.icon_button_background))
            .border_1()
            .border_color(rgb(theme.colors.icon_button_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(icon_size))
            .child("<<")
    }
}
