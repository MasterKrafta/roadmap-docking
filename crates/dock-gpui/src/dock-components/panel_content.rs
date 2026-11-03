use gpui::{IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Placeholder surface for the content hosted by a panel.
pub struct PanelContent;

impl PanelContent {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(rgb(theme.colors.panel_surface_background))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size))
            .child("Содержимое панели")
    }
}
