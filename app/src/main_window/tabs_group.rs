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

use dock_core::{Layout, NodeId};
use gpui::{App, ClickEvent, Div, ParentElement, Styled, Window, div, px, rgb};

use super::{TabDragPayload, TabDragPreview};

/// Renders any Tabs group from Core's tree.
///
/// Header visibility and floating styling are derived from the current layout:
/// the first group in a docked column has a header, groups below it do not,
/// and every floating composition has its own header and frame.
pub(super) fn render<F1, F2>(
    layout: &Layout,
    group_id: NodeId,
    theme: ui::Theme,
    active_tab: usize,
    width: f32,
    height: f32,
    first_tab: &'static str,
    second_tab: Option<&'static str>,
    first_drag: Option<TabDragPayload>,
    second_drag: Option<TabDragPayload>,
    on_first_tab: F1,
    on_second_tab: F2,
) -> Div
where
    F1: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F2: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let show_header = super::group_has_header(layout, group_id);
    let is_floating = super::group_is_floating(layout, group_id);
    let header_height = if show_header {
        theme.metrics.column_header_height.max(1.0)
    } else {
        0.0
    };
    let tab_strip_height = theme.metrics.tab_strip_height.max(1.0);
    let content_height = (height - header_height - tab_strip_height).max(1.0);
    let selected_tab = if active_tab == 1 {
        second_tab.unwrap_or(first_tab)
    } else {
        first_tab
    };

    let group = div()
        .flex()
        .flex_col()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.group_background));
    let group = if is_floating {
        group
            .border_1()
            .border_color(rgb(theme.colors.primary_text))
    } else {
        group
    };
    let group = if show_header {
        group.child(header::render(theme, width, header_height))
    } else {
        group
    };

    group
        .child(render_tab_strip(
            theme,
            active_tab,
            width,
            tab_strip_height,
            first_tab,
            second_tab,
            first_drag,
            second_drag,
            on_first_tab,
            on_second_tab,
        ))
        .child(content::render(
            theme,
            selected_tab,
            width,
            content_height,
        ))
}

/// Arranges one or two tabs, the open drag/drop area, and the hamburger button.
fn render_tab_strip<F1, F2>(
    theme: ui::Theme,
    active_tab: usize,
    width: f32,
    height: f32,
    first_tab: &'static str,
    second_tab: Option<&'static str>,
    first_drag: Option<TabDragPayload>,
    second_drag: Option<TabDragPayload>,
    on_first_tab: F1,
    on_second_tab: F2,
) -> Div
where
    F1: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F2: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let strip = div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(tab::render(
            theme,
            first_tab,
            active_tab == 0,
            height,
            on_first_tab,
            first_drag,
        ));

    let strip = if let Some(second_tab) = second_tab {
        strip.child(tab::render(
            theme,
            second_tab,
            active_tab == 1,
            height,
            on_second_tab,
            second_drag,
        ))
    } else {
        strip
    };

    strip
        .child(drag_zone::render(theme, height))
        .child(hamburger_button::render(theme, height))
}
