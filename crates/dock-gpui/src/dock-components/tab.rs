use gpui::{IntoElement, ParentElement, Styled, div, px, rgb};
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
        div()
            .id(id)
            .w(px(112.0))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .px(px(8.0))
            .overflow_hidden()
            .cursor_move()
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
