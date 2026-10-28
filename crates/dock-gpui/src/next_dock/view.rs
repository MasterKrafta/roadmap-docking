//! Rendering for the independent dock tree.

use dock_core::{Point, Rect, Size};
use gpui::{
    AnyElement, Div, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, div, px, rgb,
};
use ui::Theme;

use crate::components::{
    ClickHandler, GroupFrame, IconButton, PanelSurface, TabStrip, with_focus_ring,
};

use super::geometry::{self, GeometryKind, NodeGeometry};
use super::model::{DockLayout, DockNode, SplitAxis, TabGroup};

/// Renders the prototype shell and its current tree snapshot.
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
                grip_height,
                tab_height,
                content_height,
            },
            DockNode::Tabs(group),
        ) => render_tabs(
            group,
            geometry.width,
            geometry.height,
            *grip_height,
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
    grip_height: f32,
    tab_height: f32,
    content_height: f32,
    theme: &Theme,
    key: &str,
) -> Div {
    let group_rect = Rect::new(Point::ZERO, Size::new(width, height));

    let strip_rect = Rect::new(Point::new(0.0, grip_height), Size::new(width, tab_height));

    let content_rect = Rect::new(
        Point::new(0.0, grip_height + tab_height),
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

    let mut result = div()
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
        // TabStrip paints the entire row, including the area behind the menu button.
        .child(TabStrip {
            rect: strip_rect,
            theme: *theme,
        });

    // Visible group-grip area. Actual drag behavior will be connected separately.
    result = result.child(
        div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(width))
            .h(px(grip_height))
            .bg(rgb(theme.colors.group_grip_background)),
    );

    let grip_marker_width = width.min(24.0);
    let grip_marker_height = grip_height.min(2.0);

    if grip_marker_width > 0.0 && grip_marker_height > 0.0 {
        result = result.child(
            div()
                .absolute()
                .left(px((width - grip_marker_width) / 2.0))
                .top(px((grip_height - grip_marker_height) / 2.0))
                .w(px(grip_marker_width))
                .h(px(grip_marker_height))
                .bg(rgb(theme.colors.focus)),
        );
    }

    // Reserve a fixed area on the right for the hamburger menu.
    let menu_width = theme.metrics.group_control_button_width.min(width);
    let tabs_width = (width - menu_width).max(0.0);
    let tab_count = group.panels.len();
    let tab_width = if tab_count == 0 {
        0.0
    } else {
        tabs_width / tab_count as f32
    };

    for (index, panel) in group.panels.iter().enumerate() {
        let rect = Rect::new(
            Point::new(index as f32 * tab_width, grip_height),
            Size::new(tab_width, tab_height),
        );

        result = result.child(render_next_tab(
            format!("next-dock-tab-{key}-{index}"),
            rect,
            panel.title,
            index == group.active,
            theme,
        ));
    }

    let menu_rect = Rect::new(
        Point::new(tabs_width, grip_height),
        Size::new(menu_width, tab_height),
    );

    result.child(
        IconButton::new(
            format!("next-dock-group-menu-{key}"),
            menu_rect,
            "☰",
            theme,
            noop_click(),
        )
        .with_colors(
            theme.colors.group_menu_button,
            theme.colors.group_menu_button_hover,
        )
        .with_border(),
    )
}

/// Prototype-specific tab styling; the existing PanelTab stays unchanged for the old UI.
fn render_next_tab(
    id: String,
    rect: Rect,
    label: &str,
    active: bool,
    theme: &Theme,
) -> impl IntoElement {
    let background = if active {
        theme.colors.panel_tab_active
    } else {
        theme.colors.panel_tab_inactive
    };

    let hover_background = if active {
        background
    } else {
        theme.colors.panel_tab_hover
    };

    let typography = theme.typography;

    let tab = div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height))
        .flex()
        .items_center()
        .justify_center()
        .px(px(8.0))
        .bg(rgb(background))
        .text_color(rgb(theme.colors.primary_text))
        .font_family(typography.family)
        .text_size(px(typography.body_size))
        .line_height(px(typography.line_height))
        .child(label.to_owned());

    with_focus_ring(tab.id(id), theme)
        .hover(move |style| style.bg(rgb(hover_background)))
        .on_click(noop_click())
}

fn noop_click() -> ClickHandler {
    Box::new(|_, _, _| {})
}
