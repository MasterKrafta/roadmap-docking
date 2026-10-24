use gpui::{Div, ParentElement, Styled, div, px, rgb};

/// Placeholder content changes with the active tab.
pub(super) fn render(theme: ui::Theme, active_tab: usize, width: f32, height: f32) -> Div {
    let (title, description) = if active_tab == 0 {
        ("Контент вкладки 1", "Выбрана первая вкладка")
    } else {
        ("Контент вкладки 2", "Выбрана вторая вкладка")
    };

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
        .child(div().child(title))
        .child(
            div()
                .text_color(rgb(theme.colors.muted_text))
                .child(description),
        )
}
