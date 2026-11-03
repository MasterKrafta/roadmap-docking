use gpui::{IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Second tab header. Its full surface is reserved as a future tab drag zone.
pub struct SecondTab;

impl SecondTab {
    pub fn render(id: String, title: String, theme: Theme) -> impl IntoElement {
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
            .bg(rgb(theme.colors.panel_tab_inactive))
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
