//! Rendering for the independent dock tree.
use dock_core::{Axis, Point, Rect, Size};
use gpui::{
    AnyElement, App, Div, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Styled, WeakEntity, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::splitter_handle::ResizeStartHandler;
use crate::components::{GroupFrame, MouseDownHandler, PanelSurface, SplitterHandle};

use super::components::{ColumnHeader, GroupHeader};
use super::geometry::{self, GeometryKind, NodeGeometry};
use super::model::{DockLayout, DockNode, NodePathStep, SplitAxis, TabGroup};
use super::window::NextDockWindow;

#[derive(Clone)]
pub(super) struct SplitResizeSpec {
    pub(super) path: Vec<NodePathStep>,
    pub(super) boundary_index: usize,
    pub(super) axis: SplitAxis,
    pub(super) available_extent: f32,
    pub(super) initial_extents: Vec<f32>,
    pub(super) min_first: f32,
    pub(super) min_second: f32,
}

pub(super) fn render(
    layout: &DockLayout,
    width: f32,
    height: f32,
    window: &Window,
    theme: &Theme,
    owner: WeakEntity<NextDockWindow>,
    floating: Option<Rect>,
) -> Div {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let content_height = (height - geometry::HEADER_HEIGHT).max(1.0);

    let (tree_width, tree_height) = floating.map_or((width, content_height), |bounds| {
        (bounds.size.width, bounds.size.height)
    });

    let root_geometry = geometry::compute(&layout.root, tree_width, tree_height, window, theme);

    let content = render_node(
        &layout.root,
        &root_geometry,
        theme,
        "root",
        &[],
        window,
        &owner,
        Size::new(width, content_height),
    );

    let mut body = div()
        .relative()
        .w(px(width))
        .h(px(content_height))
        .bg(rgb(theme.colors.window_background));

    body = if let Some(bounds) = floating {
        body.child(
            div()
                .absolute()
                .left(px(bounds.origin.x))
                .top(px(bounds.origin.y))
                .w(px(bounds.size.width))
                .h(px(bounds.size.height))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child(content),
        )
    } else {
        body.child(content)
    };

    let root = div()
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
        .child(body);

    let move_owner = owner.clone();
    let up_owner = owner.clone();
    let up_out_owner = owner;

    root.on_mouse_move(
        move |event: &MouseMoveEvent, _window: &mut Window, app: &mut App| {
            if let Some(owner) = move_owner.upgrade() {
                owner.update(app, |this, cx| {
                    this.update_resize(event, cx);
                    this.update_composition_move(event, cx);
                });
            }
        },
    )
    .on_mouse_up(
        MouseButton::Left,
        move |_event: &MouseUpEvent, _window: &mut Window, app: &mut App| {
            if let Some(owner) = up_owner.upgrade() {
                owner.update(app, |this, cx| {
                    this.end_resize(cx);
                    this.end_composition_move(cx);
                });
            }
        },
    )
    .on_mouse_up_out(
        MouseButton::Left,
        move |_event: &MouseUpEvent, _window: &mut Window, app: &mut App| {
            if let Some(owner) = up_out_owner.upgrade() {
                owner.update(app, |this, cx| {
                    this.end_resize(cx);
                    this.end_composition_move(cx);
                });
            }
        },
    )
}
fn render_node(
    node: &DockNode,
    geometry: &NodeGeometry,
    theme: &Theme,
    key: &str,
    path: &[NodePathStep],
    window: &Window,
    owner: &WeakEntity<NextDockWindow>,
    viewport_size: Size,
) -> Div {
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
            GeometryKind::Column {
                header_height,
                content: content_geometry,
            },
            DockNode::Column { content },
        ) => {
            let header_rect = Rect::new(Point::ZERO, Size::new(geometry.width, *header_height));

            let mut content_path = path.to_vec();
            content_path.push(NodePathStep::ColumnContent);

            let content_key = format!("{key}-content");

            let content_view = div()
                .absolute()
                .left(px(0.0))
                .top(px(*header_height))
                .w(px(content_geometry.width))
                .h(px(content_geometry.height))
                .child(render_node(
                    content,
                    content_geometry.as_ref(),
                    theme,
                    &content_key,
                    &content_path,
                    window,
                    owner,
                    viewport_size,
                ));

            let drag_owner = owner.clone();
            let viewport_size = viewport_size;

            let on_drag_start: MouseDownHandler =
                Box::new(move |event: &MouseDownEvent, _window, app| {
                    if let Some(owner) = drag_owner.upgrade() {
                        owner.update(app, |this, _cx| {
                            this.begin_composition_move(
                                event.position,
                                viewport_size.width,
                                viewport_size.height,
                            );
                        });
                    }
                });

            div()
                .relative()
                .w(px(geometry.width))
                .h(px(geometry.height))
                .child(ColumnHeader {
                    rect: header_rect,
                    theme: *theme,
                    key: key.to_owned(),
                    on_drag_start,
                })
                .child(content_view)
        }

        (
            GeometryKind::Split {
                splitter_extent,
                children: child_geometries,
            },
            DockNode::Split { axis, children, .. },
        ) => {
            let initial_extents: Vec<f32> = child_geometries
                .iter()
                .map(|child| match axis {
                    SplitAxis::Horizontal => child.width,
                    SplitAxis::Vertical => child.height,
                })
                .collect();

            let available_extent: f32 = initial_extents.iter().sum();

            let mut split_view = match axis {
                SplitAxis::Horizontal => div()
                    .relative()
                    .flex()
                    .flex_row()
                    .w(px(geometry.width))
                    .h(px(geometry.height)),
                SplitAxis::Vertical => div()
                    .relative()
                    .flex()
                    .flex_col()
                    .w(px(geometry.width))
                    .h(px(geometry.height)),
            };

            for (index, (child, child_geometry)) in
                children.iter().zip(child_geometries).enumerate()
            {
                if index > 0 {
                    let boundary_index = index - 1;

                    let (min_first, min_second) = match axis {
                        SplitAxis::Horizontal => (
                            geometry::minimum_width(
                                &children[boundary_index],
                                theme.metrics.splitter_thickness.max(0.0),
                                window,
                                theme,
                            ),
                            geometry::minimum_width(
                                child,
                                theme.metrics.splitter_thickness.max(0.0),
                                window,
                                theme,
                            ),
                        ),
                        SplitAxis::Vertical => (80.0, 80.0),
                    };

                    let spec = SplitResizeSpec {
                        path: path.to_vec(),
                        boundary_index,
                        axis: *axis,
                        available_extent,
                        initial_extents: initial_extents.clone(),
                        min_first,
                        min_second,
                    };

                    let resize_owner = owner.clone();

                    let on_resize_start: ResizeStartHandler =
                        Box::new(move |event: &MouseDownEvent, _window, app| {
                            if let Some(owner) = resize_owner.upgrade() {
                                owner.update(app, |this, cx| {
                                    this.begin_resize(spec.clone(), event.position, cx);
                                });
                            }
                        });

                    let handle_rect = match axis {
                        SplitAxis::Horizontal => {
                            Rect::new(Point::ZERO, Size::new(*splitter_extent, geometry.height))
                        }
                        SplitAxis::Vertical => {
                            Rect::new(Point::ZERO, Size::new(geometry.width, *splitter_extent))
                        }
                    };

                    let handle = SplitterHandle {
                        id: format!("next-dock-splitter-{key}-{boundary_index}"),
                        rect: handle_rect,
                        axis: match axis {
                            SplitAxis::Horizontal => Axis::X,
                            SplitAxis::Vertical => Axis::Y,
                        },
                        theme: *theme,
                        on_resize_start: Some(on_resize_start),
                        on_resize_key: None,
                    };

                    let splitter_slot = match axis {
                        SplitAxis::Horizontal => div()
                            .relative()
                            .w(px(*splitter_extent))
                            .h(px(geometry.height))
                            .child(handle),
                        SplitAxis::Vertical => div()
                            .relative()
                            .w(px(geometry.width))
                            .h(px(*splitter_extent))
                            .child(handle),
                    };

                    split_view = split_view.child(splitter_slot);
                }

                let mut child_path = path.to_vec();
                child_path.push(NodePathStep::SplitChild(index));

                let child_key = format!("{key}-child-{index}");

                split_view = split_view.child(render_node(
                    child,
                    child_geometry,
                    theme,
                    &child_key,
                    &child_path,
                    window,
                    owner,
                    viewport_size,
                ));
            }

            split_view
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
