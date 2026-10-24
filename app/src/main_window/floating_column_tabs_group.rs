#[path = "tabs_group/close_button.rs"]
mod close_button;
#[path = "tabs_group/collapse_button.rs"]
mod collapse_button;
#[path = "tabs_group/content.rs"]
mod content;
#[path = "tabs_group/drag_zone.rs"]
mod drag_zone;
#[path = "tabs_group/hamburger_button.rs"]
mod hamburger_button;
#[path = "tabs_group/header.rs"]
mod header;
#[path = "tabs_group/tab.rs"]
mod tab;

use gpui::{App, ClickEvent, Div, ParentElement, Styled, Window, div, px, rgb};

/// Assembles one group from independent header, tab, drag-zone, button, and content components.
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
        .child(header::render(theme, width, header_height))
        .child(render_tab_strip(
            theme,
            active_tab,
            width,
            tab_strip_height,
            on_first_tab,
            on_second_tab,
        ))
        .child(content::render(
            theme,
            active_tab,
            width,
            content_height,
        ))
}

/// Arranges the two tabs, the empty drag/drop area, and the hamburger button.
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
    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(tab::render(
            theme,
            "Вкладка 1",
            active_tab == 0,
            height,
            on_first_tab,
        ))
        .child(tab::render(
            theme,
            "Вкладка 2",
            active_tab == 1,
            height,
            on_second_tab,
        ))
        .child(drag_zone::render(theme, height))
        .child(hamburger_button::render(theme, height))
}
