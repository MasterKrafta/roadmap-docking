mod tabs_groups;

use gpui::{App, ClickEvent, Div, ParentElement, Styled, Window, div, px, rgb};

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
        .child(tabs_groups::header::render(theme, width, header_height))
        .child(tabs_groups::tab_strip::render(
            theme,
            active_tab,
            width,
            tab_strip_height,
            on_first_tab,
            on_second_tab,
        ))
        .child(tabs_groups::content::render(
            theme,
            active_tab,
            width,
            content_height,
        ))
}
