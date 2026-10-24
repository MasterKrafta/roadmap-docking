use gpui::{Div, ParentElement, Styled, div, px, rgb};

/// Placeholder content changes with the active tab.
pub(super) fn render(theme: ui::Theme, active_tab: usize, width: f32, height: f32) -> Div {
    let selected_tab = if active_tab == 0 {
        "Вкладка 1"
    } else {
        "Вкладка 2"
    };
    render_selected(theme, selected_tab, width, height)
}

/// Renders placeholder content for the panel selected by the Core layout.
pub(super) fn render_selected(
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
