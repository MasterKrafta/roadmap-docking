use gpui::{Div, ParentElement, Styled, div, px, rgb};

/// Renders placeholder content for the panel selected by the Core layout.
pub(super) fn render(
    theme: ui::Theme,
    selected_tab: &'static str,
    width: f32,
    height: f32,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.group_background))
        .text_color(rgb(theme.colors.primary_text))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(div().child(format!("Контент: {selected_tab}")))
        .child(
            div()
                .text_color(rgb(theme.colors.muted_text))
                .child(format!("Выбрана {selected_tab}")),
        )
}
