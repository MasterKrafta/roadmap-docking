//! Rendering for the independent dock tree.

use dock_core::{Point, Rect, Size};
use gpui::{AnyElement, Div, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

use crate::components::{GroupFrame, PanelSurface};

use super::components::GroupHeader;
use super::geometry::{self, GeometryKind, NodeGeometry};
use super::model::{DockLayout, DockNode, SplitAxis, TabGroup};

pub(super) fn render(layout: &DockLayout, width: f32, height: f32, theme: &Theme) -> Div {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let content_height = (height - geometry::HEADER_HEIGHT).max(1.0);

    let root_geometry = geometry::compute(&layout.root, width, content_height, theme);

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
                .h(px(geometry::HEADER_HEIGHT))
                .px(px(14.0))
                .bg(rgb(theme.colors.toolbar_background))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child("NEXT DOCK  /  independent model prototype"),
        )
        .child(render_node(&layout.root, &root_geometry, theme, "root"))
}

fn render_node(node: &DockNode, geometry: &NodeGeometry, theme: &Theme, key: &str) -> Div {
    match (&geometry.kind, node) {
        (
            GeometryKind::Tabs {
                tab_height,
                content_height,
            },
            DockNode::Tabs(group),
        ) => render_tabs(
            group,
            geometry.width,
            geometry.height,
            *tab_height,
            *content_height,
            theme,
            key,
        ),

        (
            GeometryKind::Split {
                splitter_extent,
                first: first_geometry,
                second: second_geometry,
            },
            DockNode::Split {
                axis,
                first,
                second,
                ..
            },
        ) => {
            let first_key = format!("{key}-first");
            let second_key = format!("{key}-second");

            match axis {
                SplitAxis::Horizontal => div()
                    .flex()
                    .flex_row()
                    .w(px(geometry.width))
                    .h(px(geometry.height))
                    .child(render_node(first, first_geometry, theme, &first_key))
                    .child(
                        div()
                            .w(px(*splitter_extent))
                            .h(px(geometry.height))
                            .bg(rgb(theme.colors.splitter_handle)),
                    )
                    .child(render_node(second, second_geometry, theme, &second_key)),

                SplitAxis::Vertical => div()
                    .flex()
                    .flex_col()
                    .w(px(geometry.width))
                    .h(px(geometry.height))
                    .child(render_node(first, first_geometry, theme, &first_key))
                    .child(
                        div()
                            .w(px(geometry.width))
                            .h(px(*splitter_extent))
                            .bg(rgb(theme.colors.splitter_handle)),
                    )
                    .child(render_node(second, second_geometry, theme, &second_key)),
            }
        }

        _ => div()
            .w(px(geometry.width))
            .h(px(geometry.height))
            .bg(rgb(theme.colors.panel_surface_background))
            .child("Dock geometry mismatch"),
    }
}

fn render_tabs(
    group: &TabGroup,
    width: f32,
    height: f32,
    tab_height: f32,
    content_height: f32,
    theme: &Theme,
    key: &str,
) -> Div {
    let group_rect = Rect::new(Point::ZERO, Size::new(width, height));

    let strip_rect = Rect::new(Point::ZERO, Size::new(width, tab_height));

    let content_rect = Rect::new(
        Point::new(0.0, tab_height),
        Size::new(width, content_height),
    );

    let content: AnyElement = match group.panels.get(group.active) {
        Some(active) => div()
            .flex()
            .flex_col()
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
            .into_any_element(),

        None => div().child("Empty panel group").into_any_element(),
    };

    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .child(GroupFrame {
            rect: group_rect,
            theme: *theme,
        })
        .child(PanelSurface {
            rect: content_rect,
            theme: *theme,
            content,
        })
        .child(GroupHeader {
            rect: strip_rect,
            group: group.clone(),
            theme: *theme,
            key: key.to_owned(),
        })
}
