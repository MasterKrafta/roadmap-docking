use gpui::{
    App, ClickEvent, Div, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

/// Renders one tab group: header, switchable tab strip, and active tab content.
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
    let header_height = theme.metrics.column_header_height.max(1.0);
    let tab_strip_height = theme.metrics.tab_strip_height.max(1.0);
    let content_height = (height - header_height - tab_strip_height).max(1.0);

    div()
        .flex()
        .flex_col()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.group_background))
        .child(render_header(theme, width, header_height))
        .child(render_tab_strip(
            theme,
            active_tab,
            width,
            tab_strip_height,
            on_first_tab,
            on_second_tab,
        ))
        .child(render_content(
            theme,
            active_tab,
            width,
            content_height,
        ))
}

/// Empty header area with the two controls aligned on the right.
fn render_header(theme: ui::Theme, width: f32, height: f32) -> Div {
    let control_button_width = theme.metrics.group_control_button_width.max(1.0);

    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(div().flex_1().h(px(height)))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(control_button_width))
                .h(px(height))
                .bg(rgb(theme.colors.toolbar_background))
                .text_color(rgb(theme.colors.primary_text))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child(">>"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(control_button_width))
                .h(px(height))
                .bg(rgb(theme.colors.toolbar_background))
                .text_color(rgb(theme.colors.primary_text))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child("x"),
        )
}

/// Two selectable tabs, flexible free space, a reserved drag area, and the hamburger button.
fn render_tab_strip<F1, F2>(
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
    let control_button_width = theme.metrics.group_control_button_width.max(1.0);
    let drag_zone_width = theme.metrics.tab_drag_zone_width.max(0.0);

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
        // Reserved blank area for the future tab-drag/drop target.
        .child(
            div()
                .w(px(drag_zone_width))
                .h(px(height))
                .bg(rgb(theme.colors.dock_background)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(control_button_width))
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
) -> impl IntoElement
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
        .id(label)
        .on_click(on_click)
}

/// Placeholder content changes with the active tab.
fn render_content(theme: ui::Theme, active_tab: usize, width: f32, height: f32) -> Div {
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
