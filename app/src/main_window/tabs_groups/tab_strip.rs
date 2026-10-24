use gpui::{
    App, ClickEvent, Div, InteractiveElement, ParentElement, Styled, Window, div, px, rgb,
};

/// Two selectable tabs, a flexible empty drag area, and the group menu button.
pub(super) fn render<F1, F2>(
    theme: ui::Theme,
    active_tab: usize,
    width: f32,
    height: f32,
    on_first_tab: F1,
    on_second_tab: F2,
) -> Div
where
    F1: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F2: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let tab_width = theme.metrics.column_tab_width.max(1.0);

    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(tab(
            theme,
            "Вкладка 1",
            active_tab == 0,
            height,
            on_first_tab,
        ))
        .child(tab(
            theme,
            "Вкладка 2",
            active_tab == 1,
            height,
            on_second_tab,
        ))
        .child(
            div()
                .flex_1()
                .h(px(height))
                .bg(rgb(theme.colors.toolbar_background)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(34.0))
                .h(px(height))
                .bg(rgb(theme.colors.toolbar_background))
                .text_color(rgb(theme.colors.primary_text))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child("☰"),
        )
}

fn tab<F>(
    theme: ui::Theme,
    label: &'static str,
    is_active: bool,
    height: f32,
    on_click: F,
) -> Div
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let background = if is_active {
        theme.colors.active_tab
    } else {
        theme.colors.toolbar_background
    };

    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(theme.metrics.column_tab_width.max(1.0)))
        .h(px(height))
        .bg(rgb(background))
        .text_color(rgb(theme.colors.primary_text))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(label)
        .on_click(on_click)
}
