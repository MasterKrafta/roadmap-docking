use gpui::{IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Reusable tab header. The active state changes only its visual presentation;
/// pointer behavior is intentionally not connected yet.
pub struct DockTab;

impl DockTab {
    pub fn render(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
    ) -> impl IntoElement {
        Self::render_with_width(id, title, active, theme, 112.0)
    }

    pub fn render_with_width(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
        width: f32,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w(px(width))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .px(px(8.0))
            .overflow_hidden()
            .bg(rgb(if active {
                theme.colors.panel_tab_active
            } else {
                theme.colors.panel_tab_inactive
            }))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            )
    }
}
