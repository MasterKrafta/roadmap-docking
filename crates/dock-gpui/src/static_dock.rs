use std::collections::HashMap;

use dock_core::{ColumnRef, DropPayload, NodeId, PanelId, SplitterKind};
use gpui::{
    AnyElement, App, MouseButton, Pixels, Point as GpuiPoint, ScrollHandle, Window, div,
    prelude::*, px, rgb,
};
use ui::Theme;

/// Display-only projection of a tab in the current Layout.
/// This is render input, not another copy of the docking model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabItem {
    pub panel_id: PanelId,
    pub title: String,
    pub active: bool,
}

/// Display-only projection of one group, created from Layout and PanelRegistry on render.
#[derive(Clone, Debug, PartialEq)]
pub struct DockGroupData {
    pub group_id: NodeId,
    pub height: f32,
    pub tab_strip_height: f32,
    pub tabs: Vec<TabItem>,
    pub active_panel_id: Option<PanelId>,
    pub active_content_scrollable: bool,
    pub active_heading: String,
    pub active_message: String,
}

/// Render one column with a fixed header, stationary groups, and interactive splitters.
pub fn dock_column_view(
    theme: &Theme,
    width: f32,
    height: f32,
    header_height: f32,
    groups: Vec<DockGroupData>,
    panel_content_scroll_handles: &HashMap<PanelId, ScrollHandle>,
    group_splitter_indices: Vec<usize>,
    column_ref: ColumnRef,
    on_reset_column_ratios: impl Fn(ColumnRef, &mut Window, &mut App) + Clone + 'static,
    on_activate: impl Fn(NodeId, usize, &mut App) + Clone + 'static,
    on_start_drag: impl Fn(DropPayload, GpuiPoint<Pixels>, &mut Window, &mut App) + Clone + 'static,
    on_begin_splitter_interaction: impl Fn(
            SplitterKind,
            usize,
            GpuiPoint<Pixels>,
            &mut Window,
            &mut App,
        ) + Clone
        + 'static,
) -> impl IntoElement {
    let group_count = groups.len();
    let group_rows = groups.into_iter().enumerate().map(|(index, group)| {
        let splitter_height = if index + 1 < group_count {
            theme.metrics.splitter_thickness
        } else {
            0.0
        };
        let panel_content_height = (group.height - group.tab_strip_height).max(0.0);
        let group_id = group.group_id;
        let active_panel_id = group.active_panel_id;
        let content_scrollable = group.active_content_scrollable;
        let content_id = active_panel_id
            .map(|panel_id| format!("dock-panel-content-{panel_id:?}"))
            .unwrap_or_else(|| format!("dock-group-content-{group_id:?}"));
        let content_scroll_handle =
            active_panel_id.and_then(|panel_id| panel_content_scroll_handles.get(&panel_id));
        let content = panel_content(
            theme,
            width,
            panel_content_height,
            group.active_heading,
            group.active_message,
            content_id,
            content_scrollable,
            content_scroll_handle,
        );

        div()
            .id(format!("dock-group-row-{group_id:?}"))
            .w(px(width))
            .h(px((group.height + splitter_height).max(0.0)))
            .min_h(px((group.height + splitter_height).max(0.0)))
            .relative()
            .flex()
            .flex_col()
            .child(dock_group_view(
                theme,
                width,
                group.height,
                tab_strip(
                    theme,
                    width,
                    group_id,
                    group.tab_strip_height,
                    group.tabs,
                    on_activate.clone(),
                    on_start_drag.clone(),
                ),
                content,
            ))
            .child(splitter_view(theme, width, splitter_height))
            .when_some(group_splitter_indices.get(index).copied(), |row, splitter_index| {
                let on_begin = on_begin_splitter_interaction.clone();
                let on_reset = on_reset_column_ratios.clone();
                row.child(
                    div()
                        .id(format!("dock-group-splitter-hit-{splitter_index}"))
                        .absolute()
                        .left(px(0.0))
                        .top(px(
                            group.height
                                + (theme.metrics.splitter_thickness
                                    - theme.metrics.splitter_hit_zone)
                                    * 0.5,
                        ))
                        .w(px(width))
                        .h(px(theme.metrics.splitter_hit_zone))
                        .cursor_ns_resize()
                        .on_mouse_down(MouseButton::Left, move |event, window, app| {
                            if event.click_count >= 2 {
                                on_reset(column_ref, window, app);
                            } else {
                                on_begin(
                                    SplitterKind::Group,
                                    splitter_index,
                                    window.mouse_position(),
                                    window,
                                    app,
                                );
                            }
                        }),
                )
            })
    });

    div()
        .w(px(width))
        .h(px(height))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(column_header(theme, width, header_height, "Dock"))
        .children(group_rows)
}

/// Shared column header. The empty flex region is the future drag handle; actions are inert.
pub fn column_header(theme: &Theme, width: f32, height: f32, title: &str) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    div()
        .w(px(width))
        .h(px(height))
        .flex()
        .flex_row()
        .items_center()
        .px(px(metrics.content_padding))
        .bg(rgb(colors.dock_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.tab_size))
        .text_color(rgb(colors.muted_text))
        .child(div().h_full().flex().items_center().child(title.to_owned()))
        // Flexible blank space is intentionally non-interactive in R5.4.
        .child(div().flex_1().h_full())
        .child(static_button(theme, "»", colors.dock_background))
        .child(static_button(theme, "×", colors.dock_background))
}

/// A single group has its own tab strip and one active panel-content view.
pub fn dock_group_view(
    theme: &Theme,
    width: f32,
    height: f32,
    tabs: impl IntoElement,
    content: impl IntoElement,
) -> impl IntoElement {
    let colors = theme.colors;

    div()
        .w(px(width))
        .h(px(height.max(0.0)))
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(rgb(colors.group_background))
        .border_1()
        .border_color(rgb(colors.border))
        .child(tabs)
        .child(content)
}

/// Draw tabs, flexible empty space, and a disabled group-menu glyph.
pub fn tab_strip(
    theme: &Theme,
    width: f32,
    group_id: NodeId,
    height: f32,
    tabs: Vec<TabItem>,
    on_activate: impl Fn(NodeId, usize, &mut App) + Clone + 'static,
    on_start_drag: impl Fn(DropPayload, GpuiPoint<Pixels>, &mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    let tab_elements = tabs.into_iter().enumerate().map(|(index, tab)| {
        let background = if tab.active {
            colors.active_tab
        } else {
            colors.group_background
        };
        let foreground = if tab.active {
            colors.primary_text
        } else {
            colors.muted_text
        };
        let activate = on_activate.clone();
        let start_drag = on_start_drag.clone();
        let panel_id = tab.panel_id;

        div()
            .id(format!("dock-tab-{group_id:?}-{index}"))
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .flex()
            .items_center()
            .px(px(metrics.content_padding))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_size(px(typography.tab_size))
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(colors.hover)))
            .on_click(move |_, _, app| activate(group_id, index, app))
            .on_mouse_down(MouseButton::Left, move |_, window, app| {
                start_drag(
                    DropPayload::Panel(panel_id),
                    window.mouse_position(),
                    window,
                    app,
                )
            })
            .child(tab.title)
    });

    div()
        .w(px(width))
        .h(px(height.max(0.0)))
        .flex()
        .flex_row()
        .items_center()
        .px(px(metrics.content_padding))
        .bg(rgb(colors.group_background))
        .border_1()
        .border_color(rgb(colors.border))
        .children(tab_elements)
        // The blank area is the explicit capture zone for detaching the entire group.
        // Individual tabs keep their own Panel drag payloads; this handle uses Group.
        .child({
            let start_group_drag = on_start_drag.clone();
            div()
                .id(format!("dock-group-drag-handle-{group_id:?}"))
                .flex_1()
                .h_full()
                .cursor_move()
                .hover(|style| style.bg(rgb(colors.hover)))
                .on_mouse_down(MouseButton::Left, move |_, window, app| {
                    start_group_drag(
                        DropPayload::Group(group_id),
                        window.mouse_position(),
                        window,
                        app,
                    )
                })
        })
        .child(static_button(theme, "≡", colors.group_background))
}

/// Render a panel's content. Scrolling is opt-in and applies only to this content area.
pub fn panel_content(
    theme: &Theme,
    width: f32,
    height: f32,
    heading: String,
    message: String,
    content_id: String,
    scrollable: bool,
    scroll_handle: Option<&ScrollHandle>,
) -> AnyElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    let content = div()
        .id(content_id)
        .w(px(width))
        .h(px(height.max(0.0)))
        .flex()
        .flex_col()
        .px(px(metrics.content_padding))
        .py(px(metrics.content_padding))
        .bg(rgb(colors.group_background))
        .text_color(rgb(colors.primary_text))
        .child(
            div()
                .w_full()
                .flex_none()
                .text_size(px(typography.title_size))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(heading),
        )
        .child(
            div()
                .w_full()
                .flex_none()
                .text_size(px(typography.body_size))
                .text_color(rgb(colors.muted_text))
                .child(message),
        );

    match (scrollable, scroll_handle) {
        (true, Some(handle)) => content
            .overflow_y_scroll()
            .track_scroll(handle)
            .into_any_element(),
        _ => content.overflow_hidden().into_any_element(),
    }
}

/// Visual-only horizontal separator. Height and color are both theme-owned.
pub fn splitter_view(theme: &Theme, width: f32, height: f32) -> impl IntoElement {
    div()
        .w(px(width))
        .h(px(height.max(0.0)))
        .bg(rgb(theme.colors.splitter))
}

fn static_button(theme: &Theme, label: &'static str, background: u32) -> impl IntoElement {
    let metrics = theme.metrics;
    let colors = theme.colors;
    let typography = theme.typography;

    div()
        .w(px(metrics.button_size))
        .h(px(metrics.button_size))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_color(rgb(colors.muted_text))
        .text_size(px(typography.body_size))
        .child(label)
}
