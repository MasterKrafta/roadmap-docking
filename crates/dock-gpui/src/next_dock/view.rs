//! Rendering for the independent dock tree.

use gpui::{Div, ParentElement, Styled, div, px, rgb};
use ui::Theme;

use super::model::{DockLayout, DockNode, SplitAxis, TabGroup};

const HEADER_HEIGHT: f32 = 36.0;
const TAB_HEIGHT: f32 = 32.0;
const SPLITTER_SIZE: f32 = 6.0;

/// Renders the prototype shell and its current tree snapshot.
pub(super) fn render(layout: &DockLayout, width: f32, height: f32, theme: &Theme) -> Div {
    let content_height = (height - HEADER_HEIGHT).max(1.0);
    div()
        .flex()
        .flex_col()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.window_background))
        .text_color(rgb(theme.colors.primary_text))
        .child(
            div()
                .flex()
                .items_center()
                .w(px(width))
                .h(px(HEADER_HEIGHT))
                .px(px(14.0))
                .bg(rgb(theme.colors.toolbar_background))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child("NEXT DOCK  /  independent model prototype"),
        )
        .child(render_node(&layout.root, width, content_height, theme))
}

fn render_node(node: &DockNode, width: f32, height: f32, theme: &Theme) -> Div {
    match node {
        DockNode::Tabs(group) => render_tabs(group, width, height, theme),
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => match axis {
            SplitAxis::Horizontal => {
                let available = (width - SPLITTER_SIZE).max(2.0);
                let first_width = (available * ratio).clamp(1.0, available - 1.0);
                let second_width = (available - first_width).max(1.0);
                div()
                    .flex()
                    .flex_row()
                    .w(px(width))
                    .h(px(height))
                    .child(render_node(first, first_width, height, theme))
                    .child(
                        div()
                            .w(px(SPLITTER_SIZE))
                            .h(px(height))
                            .bg(rgb(theme.colors.splitter_handle)),
                    )
                    .child(render_node(second, second_width, height, theme))
            }
            SplitAxis::Vertical => {
                let available = (height - SPLITTER_SIZE).max(2.0);
                let first_height = (available * ratio).clamp(1.0, available - 1.0);
                let second_height = (available - first_height).max(1.0);
                div()
                    .flex()
                    .flex_col()
                    .w(px(width))
                    .h(px(height))
                    .child(render_node(first, width, first_height, theme))
                    .child(
                        div()
                            .w(px(width))
                            .h(px(SPLITTER_SIZE))
                            .bg(rgb(theme.colors.splitter_handle)),
                    )
                    .child(render_node(second, width, second_height, theme))
            }
        },
    }
}

fn render_tabs(group: &TabGroup, width: f32, height: f32, theme: &Theme) -> Div {
    let mut strip = div()
        .flex()
        .flex_row()
        .w(px(width))
        .h(px(TAB_HEIGHT))
        .bg(rgb(theme.colors.tab_strip_background));

    for (index, panel) in group.panels.iter().enumerate() {
        let color = if index == group.active {
            theme.colors.panel_tab_active
        } else {
            theme.colors.panel_tab_inactive
        };
        strip = strip.child(
            div()
                .flex()
                .items_center()
                .h(px(TAB_HEIGHT))
                .px(px(10.0))
                .bg(rgb(color))
                .text_color(rgb(theme.colors.primary_text))
                .child(panel.title),
        );
    }

    let body_height = (height - TAB_HEIGHT).max(1.0);
    let body = if let Some(active) = group.panels.get(group.active) {
        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(body_height))
            .p(px(14.0))
            .bg(rgb(theme.colors.panel_surface_background))
            .child(
                div()
                    .text_color(rgb(theme.colors.primary_text))
                    .child(active.title),
            )
            .child(
                div()
                    .text_color(rgb(theme.colors.muted_text))
                    .child(active.description),
            )
    } else {
        div()
            .w(px(width))
            .h(px(body_height))
            .bg(rgb(theme.colors.panel_surface_background))
            .child("Empty panel group")
    };

    div()
        .flex()
        .flex_col()
        .w(px(width))
        .h(px(height))
        .border_1()
        .border_color(rgb(theme.colors.group_frame_border))
        .bg(rgb(theme.colors.group_frame_background))
        .child(strip)
        .child(body)
}
