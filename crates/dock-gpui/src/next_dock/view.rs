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
use super::model::{CompositionContainer, DockLayout, DockNode, NodePathStep, SplitAxis, TabGroup};
use super::window::{
    CompositionDropZone, FloatingComposition, GroupDragPreview, NextDockWindow, TabDragContext,
    TabDragPreview, TabReorderPreview,
};

#[derive(Clone)]
pub(super) struct SplitResizeSpec {
    pub(super) composition_index: usize,
    pub(super) path: Vec<NodePathStep>,
    pub(super) boundary_index: usize,
    pub(super) axis: SplitAxis,
    pub(super) available_extent: f32,
    pub(super) initial_extents: Vec<f32>,
    pub(super) min_first: f32,
    pub(super) min_second: f32,
}

pub(super) fn render(
    container: &CompositionContainer,
    width: f32,
    height: f32,
    window: &Window,
    theme: &Theme,
    owner: WeakEntity<NextDockWindow>,
    floating_compositions: &[FloatingComposition],
    drop_zone: Option<CompositionDropZone>,
    tab_drag_preview: Option<TabDragPreview>,
    group_drag_preview: Option<GroupDragPreview>,
    tab_reorder_preview: Option<TabReorderPreview>,
) -> Div {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let content_height = (height - geometry::HEADER_HEIGHT).max(1.0);

    let docked_slots: Vec<(usize, &super::model::CompositionSlot)> = container
        .slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            (!floating_compositions
                .iter()
                .any(|floating| floating.composition_index == index))
            .then_some((index, slot))
        })
        .collect();

    let splitter_extent = theme.metrics.splitter_thickness.max(0.0);
    let splitter_count = docked_slots.len().saturating_sub(1);
    let available_width = (width - splitter_extent * splitter_count as f32).max(0.0);

    let weights: Vec<f32> = docked_slots
        .iter()
        .map(|(_, slot)| slot.weight.max(0.0))
        .collect();

    let widths = distribute_composition_widths(available_width, &weights);

    let mut docked_row = div().flex().flex_row().w(px(width)).h(px(content_height));
    let mut slot_origin_x = 0.0;

    for (position, ((composition_index, slot), slot_width)) in
        docked_slots.iter().zip(widths.iter()).enumerate()
    {
        if position > 0 {
            slot_origin_x += splitter_extent;

            docked_row = docked_row.child(
                div()
                    .w(px(splitter_extent))
                    .h(px(content_height))
                    .bg(rgb(theme.colors.splitter_handle)),
            );
        }

        let slot_origin = Point::new(slot_origin_x, 0.0);

        let slot_view = match slot.layout.as_ref() {
            Some(layout) => {
                let node_geometry =
                    geometry::compute(&layout.root, *slot_width, content_height, window, theme);

                render_node(
                    &layout.root,
                    &node_geometry,
                    theme,
                    &format!("composition-{composition_index}"),
                    &[],
                    window,
                    &owner,
                    Size::new(width, content_height),
                    *composition_index,
                    true,
                    slot_origin,
                    group_drag_preview.as_ref(),
                    tab_reorder_preview.as_ref(),
                )
            }
            None => div()
                .w(px(*slot_width))
                .h(px(content_height))
                .bg(rgb(theme.colors.panel_surface_background))
                .border_1()
                .border_color(rgb(theme.colors.border)),
        };

        docked_row = docked_row.child(
            div()
                .w(px(*slot_width))
                .h(px(content_height))
                .child(slot_view),
        );

        slot_origin_x += *slot_width;
    }

    let mut body = div()
        .relative()
        .w(px(width))
        .h(px(content_height))
        .bg(rgb(theme.colors.window_background))
        .child(docked_row);

    for floating in floating_compositions {
        if let Some(layout) = container
            .slots
            .get(floating.composition_index)
            .and_then(|slot| slot.layout.as_ref())
        {
            let bounds = floating.bounds;
            let node_geometry = geometry::compute(
                &layout.root,
                bounds.size.width,
                bounds.size.height,
                window,
                theme,
            );

            let floating_content = render_node(
                &layout.root,
                &node_geometry,
                theme,
                &format!("composition-{}-floating", floating.composition_index),
                &[],
                window,
                &owner,
                Size::new(width, content_height),
                floating.composition_index,
                true,
                bounds.origin,
                group_drag_preview.as_ref(),
                tab_reorder_preview.as_ref(),
            );

            body = body.child(
                div()
                    .absolute()
                    .left(px(bounds.origin.x))
                    .top(px(bounds.origin.y))
                    .w(px(bounds.size.width))
                    .h(px(bounds.size.height))
                    .border_1()
                    .border_color(rgb(theme.colors.border))
                    .child(floating_content),
            );
        }
    }

    if let Some(preview) = tab_drag_preview {
        let preview_layout = DockLayout::single_panel(preview.panel);

        let node_geometry = geometry::compute(
            &preview_layout.root,
            preview.bounds.size.width,
            preview.bounds.size.height,
            window,
            theme,
        );

        let preview_content = render_node(
            &preview_layout.root,
            &node_geometry,
            theme,
            "tab-drag-preview",
            &[],
            window,
            &owner,
            Size::new(width, content_height),
            0,
            false,
            preview.bounds.origin,
            None,
            tab_reorder_preview.as_ref(),
        );

        body = body.child(
            div()
                .absolute()
                .left(px(preview.bounds.origin.x))
                .top(px(preview.bounds.origin.y))
                .w(px(preview.bounds.size.width))
                .h(px(preview.bounds.size.height))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child(preview_content),
        );
    }

    if let Some(preview) = group_drag_preview.as_ref() {
        let bounds = preview.bounds;
        let preview_layout = DockLayout::single_group(preview.group.clone());
        let node_geometry = geometry::compute(
            &preview_layout.root,
            bounds.size.width,
            bounds.size.height,
            window,
            theme,
        );

        let preview_content = render_node(
            &preview_layout.root,
            &node_geometry,
            theme,
            "group-drag-preview",
            &[],
            window,
            &owner,
            Size::new(width, content_height),
            0,
            false,
            bounds.origin,
            None,
            tab_reorder_preview.as_ref(),
        );

        body = body.child(
            div()
                .absolute()
                .left(px(bounds.origin.x))
                .top(px(bounds.origin.y))
                .w(px(bounds.size.width))
                .h(px(bounds.size.height))
                .border_1()
                .border_color(rgb(theme.colors.border))
                .child(preview_content),
        );
    }

    if let Some(drop_zone) = drop_zone {
        let zone_width = 12.0;
        let zone_x = match drop_zone {
            CompositionDropZone::LeftEdge => 0.0,
            CompositionDropZone::RightEdge => (width - zone_width).max(0.0),
        };

        body = body.child(
            div()
                .absolute()
                .left(px(zone_x))
                .top(px(0.0))
                .w(px(zone_width))
                .h(px(content_height))
                .bg(rgb(theme.colors.splitter_handle))
                .border_1()
                .border_color(rgb(theme.colors.border)),
        );
    }

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
                    this.update_tab_drag(event, cx);
                    this.update_group_drag(event, cx);
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
                    this.end_group_drag(cx);
                    this.end_tab_drag(cx);
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
                    this.end_group_drag(cx);
                    this.end_tab_drag(cx);
                    this.end_composition_move(cx);
                });
            }
        },
    )
}

fn distribute_composition_widths(available: f32, weights: &[f32]) -> Vec<f32> {
    if weights.is_empty() {
        return Vec::new();
    }

    let total_weight: f32 = weights.iter().sum();

    let mut widths = if total_weight <= f32::EPSILON {
        vec![available / weights.len() as f32; weights.len()]
    } else {
        weights
            .iter()
            .map(|weight| available * weight / total_weight)
            .collect()
    };

    let allocated: f32 = widths.iter().sum();

    if let Some(last) = widths.last_mut() {
        *last += available - allocated;
    }

    widths
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
    composition_index: usize,
    interactive: bool,
    origin: Point,
    group_drag_preview: Option<&GroupDragPreview>,
    tab_reorder_preview: Option<&TabReorderPreview>,
) -> Div {
    if group_drag_preview.is_some_and(|preview| {
        preview.composition_index == composition_index
            && (preview.group_key.as_str() == key
                || preview.group_key.strip_suffix("-content") == Some(key))
    }) {
        return div()
            .w(px(geometry.width))
            .h(px(geometry.height))
            .bg(rgb(theme.colors.panel_surface_background));
    }

    match (&geometry.kind, node) {
        (
            GeometryKind::Tabs {
                tab_height,
                content_height,
            },
            DockNode::Tabs(group),
        ) => {
            let group_tab_reorder_preview = tab_reorder_preview
                .filter(|preview| {
                    preview.composition_index == composition_index
                        && preview.group_key.as_str() == key
                })
                .cloned();

            let dragged_panel_index = group_tab_reorder_preview
                .as_ref()
                .map(|preview| preview.panel_index);
            let tab_drag_context = interactive.then(|| TabDragContext {
                composition_index,
                path: path.to_vec(),
                viewport_size,
                owner: owner.clone(),
                group_key: key.to_owned(),
                dragged_panel_index,
                strip_bounds: Rect::new(origin, Size::new(geometry.width, *tab_height)),
            });

            render_tabs(
                group,
                geometry.width,
                geometry.height,
                *tab_height,
                *content_height,
                theme,
                key,
                tab_drag_context,
                group_tab_reorder_preview,
            )
        }

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
                    composition_index,
                    interactive,
                    Point::new(origin.x, origin.y + *header_height),
                    group_drag_preview,
                    tab_reorder_preview,
                ));

            let drag_owner = owner.clone();
            let viewport_size = viewport_size;

            let on_drag_start: MouseDownHandler = if interactive == true {
                Box::new(move |event: &MouseDownEvent, _window, app| {
                    if let Some(owner) = drag_owner.upgrade() {
                        owner.update(app, |this, _cx| {
                            this.begin_composition_move(
                                composition_index,
                                event.position,
                                viewport_size.width,
                                viewport_size.height,
                            );
                        });
                    }
                })
            } else {
                Box::new(|_, _, _| {})
            };

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

            let mut child_origin = origin;

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
                        composition_index,
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

                    child_origin = match axis {
                        SplitAxis::Horizontal => {
                            Point::new(child_origin.x + *splitter_extent, child_origin.y)
                        }
                        SplitAxis::Vertical => {
                            Point::new(child_origin.x, child_origin.y + *splitter_extent)
                        }
                    };
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
                    composition_index,
                    interactive,
                    child_origin,
                    group_drag_preview,
                    tab_reorder_preview,
                ));

                child_origin = match axis {
                    SplitAxis::Horizontal => {
                        Point::new(child_origin.x + child_geometry.width, child_origin.y)
                    }
                    SplitAxis::Vertical => {
                        Point::new(child_origin.x, child_origin.y + child_geometry.height)
                    }
                };
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
    tab_drag_context: Option<TabDragContext>,
    tab_reorder_preview: Option<TabReorderPreview>,
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
            tab_drag_context,
            tab_reorder_preview,
        })
}
