use std::collections::{HashMap, HashSet};

use crate::components::{
    DropZone, FloatingTitle, FlyoutHeader, GroupFrame, GroupGrip, GroupMenuButton,
    PanelSurface, PanelTab, RailEntry, RectBox, ResizeEdgeHandle, ResizeEdgeKind, SplitterHandle,
    TabOverflowButton, TabStrip,
};

use crate::chrome_geometry::{GROUP_DRAG_GRIP_HEIGHT, GROUP_MENU_BUTTON_WIDTH};
use crate::frame::FrameGeometry;
use crate::tree_query::{
    active_panel, drop_target_node, first_panel_in_subtree, main_column_ids, subtree_nodes,
};
use crate::geometry::{localize_tree_rects, scroll_rect_in_viewport, translate_rect_to_local};
use crate::labels::tab_header_label;

use dock_core::{
    Axis, DockCommand, DockSide, DragHandle, DragPayload, DragPreview,
    DropZoneRect, FloatingGroupId, Layout, LayoutRects, Node, NodeId, PanelId, PanelInstance,
    Point, Rect, ResizeEdge, Size,
};
use gpui::{
    AnyElement, App, ClickEvent, Div, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, ScrollHandle, StatefulInteractiveElement, Styled, Window, div,
    px, rgb,
};
use ui::Theme;

/// A GPUI click handler created by the app for a command or a menu request.
pub type CommandClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Begins a pure Core drag session from a UI interaction surface.
pub type DragStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Begins a resize gesture from Core-provided geometry.
pub type ResizeStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Semantic resize handle emitted by the GPUI adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeHandle {
    Splitter { owner: NodeId, boundary_index: usize, axis: Axis },
    OuterColumn { column: NodeId, side: DockSide },
    FloatingEdge { group: FloatingGroupId, edge: ResizeEdge },
}

/// Source-aware, transient overlay produced from Core's drag preview and drop-zone geometry.
#[derive(Clone, Debug)]
pub struct DragOverlay {
    pub preview: DragPreview,
    pub zones: Vec<DropZoneRect>,
}

/// Requests app-owned menu state without transferring ownership of the layout to the adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockMenuRequest {
    TabOverflow { group: NodeId },
    PanelGroup { group: NodeId, panel: Option<PanelId> },
}

/// Renders the Core layout using the centralized panel registry for titles, icons and content.
///
/// The existing callback-based `render` remains available during migration, while
/// this entrypoint ensures panel presentation comes from `panels::PANELS`.
#[allow(clippy::too_many_arguments)]
pub fn render_from_registry<H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    render(
        layout,
        rects,
        scroll_handles,
        theme,
        crate::panels::render_panel,
        crate::panels::panel_title,
        crate::panels::panel_icon,
        make_command_handler,
        make_menu_handler,
        make_drag_start_handler,
        make_resize_start_handler,
        drag_overlay,
    )
}

/// Legacy callback renderer that accepts a caller-supplied geometry snapshot.
///
/// New code should prefer `render_frame_from_registry`, which consumes the immutable,
/// normalized geometry and caches captured by `FrameGeometry::compute`.
#[allow(clippy::too_many_arguments)]
pub fn render<F, T, I, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    render_panel: F,
    panel_title: T,
    panel_icon: I,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    I: Fn(&PanelInstance) -> &'static str,
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    render_impl(
        layout,
        rects,
        None,
        scroll_handles,
        theme,
        render_panel,
        panel_title,
        panel_icon,
        make_command_handler,
        make_menu_handler,
        make_drag_start_handler,
        make_resize_start_handler,
        drag_overlay,
    )
}

/// Renders from one immutable `FrameGeometry` snapshot and uses its per-frame caches.
///
/// Use `frame.drop_zones(layout, source, metrics)` to calculate matching drop targets.
#[allow(clippy::too_many_arguments)]
pub fn render_frame_from_registry<H, M, D, R>(
    layout: &Layout,
    frame: &FrameGeometry,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    render_impl(
        layout,
        frame.rects(),
        Some(frame),
        scroll_handles,
        theme,
        crate::panels::render_panel,
        crate::panels::panel_title,
        crate::panels::panel_icon,
        make_command_handler,
        make_menu_handler,
        make_drag_start_handler,
        make_resize_start_handler,
        drag_overlay,
    )
}


/// Internal implementation shared by the rectangle and frame-aware render entrypoints.
fn render_impl<F, T, I, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    frame: Option<&FrameGeometry>,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    theme: &Theme,
    mut render_panel: F,
    panel_title: T,
    panel_icon: I,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    I: Fn(&PanelInstance) -> &'static str,
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    let mut root = div()
        .relative()
        .w(px(rects.viewport.map_or(1.0, |value| value.size.width)))
        .h(px(rects.viewport.map_or(1.0, |value| value.size.height)))
        .bg(rgb(theme.colors.window_background));

    if let Some(canvas) = rects.canvas {
        root = root.child(RectBox::new(canvas, theme.colors.canvas_background));
    }

    // Main dock areas are painted first; floating areas are layered above them.
    // Core's overflow records decide which columns get an actual scroll viewport.
    let mut main_column_overflows = Vec::new();
    for side in [DockSide::Left, DockSide::Right] {
        let Some(area) = layout.main_area(side) else {
            continue;
        };
        let nodes = nodes_for_root(layout, area.root, frame);
        let overflow_columns: Vec<_> = rects
            .vertical_overflow
            .iter()
            .filter(|overflow| {
                nodes.contains(&overflow.owner)
                    && matches!(layout.node(overflow.owner), Some(Node::Column(_)))
            })
            .copied()
            .collect();

        let mut non_scrolling_nodes = nodes;
        for overflow in &overflow_columns {
            let column_nodes = nodes_for_root(layout, overflow.owner, frame);
            for node in &column_nodes {
                non_scrolling_nodes.remove(node);
            }
            main_column_overflows.push((*overflow, column_nodes));
        }

        if !non_scrolling_nodes.is_empty() {
            for element in tree_elements(
                layout,
                rects,
                frame,
                &non_scrolling_nodes,
                theme,
                &mut render_panel,
                &panel_title,
                &panel_icon,
                &make_command_handler,
                &make_menu_handler,
                &make_drag_start_handler,
                &make_resize_start_handler,
            ) {
                root = root.child(element);
            }
        }

        for overflow in overflow_columns {
            let Some(scroll_handle) = scroll_handles.get(&overflow.owner) else {
                continue;
            };
            let column_nodes = nodes_for_root(layout, overflow.owner, frame);
            let mut local_rects = localize_tree_rects(rects, overflow.viewport.origin);
            let Some(column_rect) = local_rects
                .columns
                .iter_mut()
                .find(|column| column.column == overflow.owner)
            else {
                continue;
            };
            // The viewport is the scroll container; Core's content extent sets the inner
            // column size, including rail entries that continue below the viewport.
            column_rect.rect =
                translate_rect_to_local(overflow.content_extent, overflow.viewport.origin);

            let mut content = div()
                .relative()
                .w(px(overflow.content_extent.size.width))
                .h(px(overflow.content_extent.size.height));
            for element in tree_elements(
                layout,
                &local_rects,
                frame,
                &column_nodes,
                theme,
                &mut render_panel,
                &panel_title,
                &panel_icon,
                &make_command_handler,
                &make_menu_handler,
                &make_drag_start_handler,
                &make_resize_start_handler,
            ) {
                content = content.child(element);
            }

            root = root.child(
                rect_element(overflow.viewport, theme.colors.dock_background)
                    .id(format!("dock-column-scroll-{:?}", overflow.owner))
                    .overflow_y_scroll()
                    .track_scroll(scroll_handle)
                    .child(content)
                    .into_any_element(),
            );
        }
    }

    // Canvas-facing edges resize the outer column through ResizeColumn.
    for side in [DockSide::Left, DockSide::Right] {
        let columns = main_column_ids(layout, side);
        let column_id = if side == DockSide::Left {
            columns.last().copied()
        } else {
            columns.first().copied()
        };
        let Some(column_id) = column_id else {
            continue;
        };
        if !matches!(layout.node(column_id), Some(Node::Column(column)) if !column.mode.is_rail()) {
            continue;
        }
        let Some(column_rect) = rects.columns.iter().find(|entry| entry.column == column_id) else {
            continue;
        };
        let width = theme.metrics.splitter_hit_zone.min(column_rect.rect.size.width);
        let edge_x = if side == DockSide::Left {
            column_rect.rect.origin.x + column_rect.rect.size.width
        } else {
            column_rect.rect.origin.x
        };
        let handle_rect = Rect::new(
            Point::new(edge_x - width / 2.0, column_rect.rect.origin.y),
            Size::new(width, column_rect.rect.size.height),
        );
        root = root.child(
            ResizeEdgeHandle {
                id: format!("dock-outer-column-resize-{:?}", column_id),
                rect: handle_rect,
                edge: ResizeEdgeKind::OuterColumn,
                theme: *theme,
                on_resize_start: make_resize_start_handler(ResizeHandle::OuterColumn {
                    column: column_id,
                    side,
                }),
            }
            .into_any_element(),
        );
    }

    for (floating_id, floating) in layout.floating_groups_in_paint_order() {
        let Some(rect) = rects
            .floating_groups
            .iter()
            .find(|candidate| candidate.group == floating_id)
            .map(|candidate| candidate.rect)
        else {
            continue;
        };
        let Some(area) = layout.dock_area(floating.area) else {
            continue;
        };

        // Floating stacks are rendered as a single clipped container. Core geometry stays
        // in window coordinates; a translated snapshot lets its descendants render locally.
        let local_rects = localize_tree_rects(rects, rect.origin);
        let nodes = nodes_for_root(layout, area.root, frame);
        let mut container = rect_element(rect, theme.colors.dock_background)
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme.colors.border));

        for element in tree_elements(
            layout,
            &local_rects,
            frame,
            &nodes,
            theme,
            &mut render_panel,
            &panel_title,
            &panel_icon,
            &make_command_handler,
            &make_menu_handler,
            &make_drag_start_handler,
            &make_resize_start_handler,
        ) {
            container = container.child(element);
        }

        let title = first_panel_in_subtree(layout, area.root)
            .and_then(|panel| layout.panel_instance(panel))
            .map(&panel_title)
            .unwrap_or_else(|| "Floating group".to_owned());
        let title_height = theme.metrics.column_header_height.min(rect.size.height);
        let title_rect = Rect::new(Point::ZERO, Size::new(rect.size.width, title_height));
        let payload = DragPayload::FloatingGroup { group: floating_id };
        container = container.child(
            FloatingTitle {
                group: floating_id,
                rect: title_rect,
                title,
                theme: *theme,
                on_drag_start: make_drag_start_handler(payload, DragHandle::FloatingTitle),
                on_close: None,
                on_collapse: None,
            }
            .into_any_element(),
        );
        root = root.child(container.into_any_element());
    }

    // Floating resize hit zones come from Core geometry. Put corners above long edges.
    for handle in rects.floating_resize_edges.iter().rev() {
        root = root.child(
            ResizeEdgeHandle {
                id: format!("dock-floating-resize-{:?}", handle.group),
                rect: handle.rect,
                edge: ResizeEdgeKind::Floating(handle.edge),
                theme: *theme,
                on_resize_start: make_resize_start_handler(ResizeHandle::FloatingEdge {
                    group: handle.group,
                    edge: handle.edge,
                }),
            }
            .into_any_element(),
        );
    }

    // The flyout is one absolute-positioned overlay. Its children use local coordinates,
    // so the overlay never participates in the Canvas/dock layout or changes base rectangles.
    if let Some(flyout) = rects.flyout
        && let Some(instance) = layout.panel_instance(flyout.panel)
    {
        let header_height = theme.metrics.column_header_height.min(flyout.rect.size.height);
        let header_rect = Rect::new(
            Point::ZERO,
            Size::new(flyout.rect.size.width, header_height),
        );
        let header = FlyoutHeader {
            group: flyout.group,
            panel: flyout.panel,
            rect: header_rect,
            title: panel_title(instance),
            theme: *theme,
            on_menu: make_menu_handler(DockMenuRequest::PanelGroup {
                group: flyout.group,
                panel: Some(flyout.panel),
            }),
            on_close: make_command_handler(DockCommand::CloseFlyout),
        };

        let content_rect = Rect::new(
            Point::new(0.0, header_height),
            Size::new(
                flyout.rect.size.width,
                (flyout.rect.size.height - header_height).max(0.0),
            ),
        );
        let payload = DragPayload::Panel { panel: flyout.panel };
        let content = rect_element(content_rect, theme.colors.group_background)
            .child(render_panel(instance))
            .id(format!("dock-flyout-panel-{:?}", flyout.panel))
            .on_mouse_down(
                MouseButton::Left,
                make_drag_start_handler(payload, DragHandle::FlyoutPanel),
            );

        let overlay = rect_element(flyout.rect, theme.colors.dock_background)
            .border_1()
            .border_color(rgb(theme.colors.border))
            .child(header.into_any_element())
            .child(content.into_any_element());
        root = root.child(overlay.into_any_element());
    }

    if let Some(overlay) = drag_overlay {
        for zone in &overlay.zones {
            let mut zone_rect = zone.rect;
            if let Some(target_node) = drop_target_node(zone.target)
                && let Some((overflow, _)) = main_column_overflows
                    .iter()
                    .find(|(_, nodes)| nodes.contains(&target_node))
            {
                let scroll_offset_y = scroll_handles
                    .get(&overflow.owner)
                    .map_or(0.0, |handle| handle.offset().y.as_f32());
                let Some(scrolled_rect) =
                    scroll_rect_in_viewport(zone.rect, overflow.viewport, scroll_offset_y)
                else {
                    continue;
                };
                zone_rect = scrolled_rect;
            }

            let active = overlay.preview.target == Some(zone.target);
            root = root.child(
                DropZone {
                    rect: zone_rect,
                    active,
                    theme: *theme,
                }
                .into_any_element(),
            );
        }
        let marker = Rect::new(
            Point::new(
                (overlay.preview.point.x - 5.0).max(0.0),
                (overlay.preview.point.y - 5.0).max(0.0),
            ),
            Size::new(10.0, 10.0),
        );
        root = root.child(
            rect_element(marker, theme.colors.focus)
                .border_1()
                .border_color(rgb(theme.colors.primary_text))
                .into_any_element(),
        );
    }

    root
}


fn nodes_for_root(
    layout: &Layout,
    root: NodeId,
    frame: Option<&FrameGeometry>,
) -> HashSet<NodeId> {
    frame
        .and_then(|frame| frame.nodes_in_root(root).cloned())
        .unwrap_or_else(|| subtree_nodes(layout, root))
}


fn tree_elements<F, T, I, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    frame: Option<&FrameGeometry>,
    nodes: &HashSet<NodeId>,
    theme: &Theme,
    render_panel: &mut F,
    panel_title: &T,
    panel_icon: &I,
    make_command_handler: &H,
    make_menu_handler: &M,
    make_drag_start_handler: &D,
    make_resize_start_handler: &R,
) -> Vec<AnyElement>
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    I: Fn(&PanelInstance) -> &'static str,
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    let mut elements = Vec::new();
    let mut computed_rail_groups = HashSet::new();
    if frame.is_none() {
        for column_rect in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
            if matches!(
                layout.node(column_rect.column),
                Some(Node::Column(column)) if column.mode.is_rail()
            ) && let Ok(items) = layout.column_display_order(column_rect.column)
            {
                computed_rail_groups.extend(items.into_iter().map(|item| item.group));
            }
        }
    }
    let rail_groups = frame
        .map(FrameGeometry::rail_groups)
        .unwrap_or(&computed_rail_groups);

    for column in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        elements.push(
            RectBox::new(column.rect, theme.colors.dock_background).into_any_element(),
        );
    }

    for group in rects.groups.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            GroupFrame {
                rect: group.rect,
                theme: *theme,
            }
            .into_any_element(),
        );

        // A 4 px drag grip is emitted only when this group's strip was moved
        // into the former header row. A floating window's top group uses its
        // actual title bar instead.
        if let Some(Node::Tabs(tabs)) = layout.node(group.group)
            && !tabs.collapsed
            && let Some(strip) = rects.tab_strips.iter().find(|strip| strip.group == group.group)
        {
            let grip_height = GROUP_DRAG_GRIP_HEIGHT
                .min(theme.metrics.column_header_height)
                .min(group.rect.size.height);
            let strip_offset = strip.rect.origin.y - group.rect.origin.y;
            if grip_height > 0.0 && (strip_offset - grip_height).abs() < 0.01 {
                let payload = DragPayload::Group { group: group.group };
                let grip_rect = Rect::new(
                    group.rect.origin,
                    Size::new(group.rect.size.width, grip_height),
                );
                elements.push(
                    GroupGrip {
                        group: group.group,
                        rect: grip_rect,
                        theme: *theme,
                        on_drag_start: make_drag_start_handler(payload, DragHandle::GroupGrip),
                    }
                    .into_any_element(),
                );
            }
        }
    }

    // The strip background is presentation only. Group dragging belongs to
    // the separate thin grip above it; menu and tab controls remain independent.
    for strip in rects.tab_strips.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            TabStrip {
                rect: strip.rect,
                theme: *theme,
            }
            .into_any_element(),
        );
    }

    for header in rects
        .tab_headers
        .iter()
        .filter(|entry| nodes.contains(&entry.group) && !rail_groups.contains(&entry.group))
    {
        let collapsed = matches!(
            layout.node(header.group),
            Some(Node::Tabs(tabs)) if tabs.collapsed
        );
        let label = layout
            .panel_instance(header.panel)
            .map(panel_title)
            .unwrap_or_else(|| "Missing panel".to_owned());
        let display_label = tab_header_label(&label, collapsed, header.index);
        let active = matches!(
            layout.node(header.group),
            Some(Node::Tabs(tabs)) if active_panel(layout, tabs) == Some(header.panel)
        );
        let payload = DragPayload::Panel { panel: header.panel };
        let activate_handler = make_command_handler(DockCommand::ActivateTab {
            group: header.group,
            panel: header.panel,
        });
        let collapse_handler = make_command_handler(DockCommand::CollapseGroup {
            group: header.group,
            collapsed: !collapsed,
        });
        elements.push(
            PanelTab {
                id: format!("dock-tab-{:?}-{:?}", header.group, header.panel),
                rect: header.rect,
                label: display_label,
                active,
                theme: *theme,
                on_activate: activate_handler,
                on_collapse_toggle: collapse_handler,
                on_drag_start: make_drag_start_handler(payload, DragHandle::PanelTab),
            }
            .into_any_element(),
        );
    }

    for overflow in rects
        .tab_overflows
        .iter()
        .filter(|entry| nodes.contains(&entry.group))
    {
        elements.push(
            TabOverflowButton {
                group: overflow.group,
                rect: overflow.button_rect,
                theme: *theme,
                on_open: make_menu_handler(DockMenuRequest::TabOverflow {
                    group: overflow.group,
                }),
            }
            .into_any_element(),
        );
    }

    for strip in rects
        .tab_strips
        .iter()
        .filter(|entry| nodes.contains(&entry.group) && !rail_groups.contains(&entry.group))
    {
        let Some(Node::Tabs(tabs)) = layout.node(strip.group) else {
            continue;
        };
        let button_width = GROUP_MENU_BUTTON_WIDTH.min(strip.rect.size.width);
        let menu_rect = Rect::new(
            Point::new(
                strip.rect.origin.x + strip.rect.size.width - button_width,
                strip.rect.origin.y,
            ),
            Size::new(button_width, strip.rect.size.height),
        );
        elements.push(
            GroupMenuButton {
                group: strip.group,
                panel: active_panel(layout, tabs),
                rect: menu_rect,
                theme: *theme,
                on_open: make_menu_handler(DockMenuRequest::PanelGroup {
                    group: strip.group,
                    panel: active_panel(layout, tabs),
                }),
            }
            .into_any_element(),
        );
    }

    // Core emits content rectangles only for visible active panels. Call the
    // app-owned factory only after confirming that a panel belongs to this area.
    // FrameGeometry already contains the panel-to-node index, avoiding a full
    // Layout walk for every rendered area in the frame.
    let legacy_panel_nodes = frame.is_none().then(|| {
        layout
            .nodes()
            .filter_map(|(node_id, node)| match node {
                Node::Panel(panel) => Some((panel.panel_id, node_id)),
                _ => None,
            })
            .collect::<HashMap<PanelId, NodeId>>()
    });
    for panel_rect in rects.panels.iter().filter(|entry| {
        let node_id = frame
            .and_then(|frame| frame.panel_node(entry.panel))
            .or_else(|| {
                legacy_panel_nodes
                    .as_ref()
                    .and_then(|nodes| nodes.get(&entry.panel).copied())
            });
        node_id.is_some_and(|node_id| nodes.contains(&node_id))
    }) {
        let Some(instance) = layout.panel_instance(panel_rect.panel) else {
            continue;
        };
        elements.push(
            PanelSurface {
                rect: panel_rect.rect,
                theme: *theme,
                content: render_panel(instance),
            }
            .into_any_element(),
        );
    }

    for splitter in rects
        .splitters
        .iter()
        .filter(|entry| nodes.contains(&entry.owner))
    {
        let handle = supported_splitter_handle(
            layout,
            splitter.owner,
            splitter.boundary_index,
            splitter.axis,
        );
        elements.push(
            SplitterHandle {
                id: format!("dock-splitter-{:?}-{}", splitter.owner, splitter.boundary_index),
                rect: splitter.rect,
                axis: splitter.axis,
                theme: *theme,
                on_resize_start: handle.map(&make_resize_start_handler),
            }
            .into_any_element(),
        );
    }

    // Rail columns flatten their panel list for presentation while retaining the Core tree.
    // Use Core's rectangles rather than re-deriving slot height/position in GPUI.
    for column_rect in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        let Some(Node::Column(column)) = layout.node(column_rect.column) else {
            continue;
        };
        if !column.mode.is_rail() {
            continue;
        }
        let Ok(items) = layout.column_display_order(column_rect.column) else {
            continue;
        };
        let selected_flyout = layout
            .flyout()
            .filter(|flyout| flyout.column == column_rect.column)
            .map(|flyout| flyout.panel);
        let visible_bottom = column_rect.rect.origin.y + column_rect.rect.size.height;

        for item in items {
            let Some(header) = rects
                .tab_headers
                .iter()
                .find(|header| header.group == item.group && header.panel == item.panel)
            else {
                continue;
            };
            if header.rect.origin.y >= visible_bottom {
                break;
            }
            let height = header
                .rect
                .size
                .height
                .min(visible_bottom - header.rect.origin.y);
            if height <= 0.0 {
                continue;
            }
            let item_rect = Rect::new(
                header.rect.origin,
                Size::new(header.rect.size.width, height),
            );
            let Some(instance) = layout.panel_instance(item.panel) else {
                continue;
            };
            let label = panel_title(instance);
            let icon = panel_icon(instance);
            let active_in_model = matches!(
                layout.node(item.group),
                Some(Node::Tabs(tabs)) if active_panel(layout, tabs) == Some(item.panel)
            );
            let active = selected_flyout.map_or(active_in_model, |panel| panel == item.panel);
            elements.push(
                RailEntry {
                    id: format!("dock-rail-{:?}-{:?}", column_rect.column, item.panel),
                    rect: item_rect,
                    mode: column.mode,
                    icon: icon.to_owned(),
                    title: label,
                    active,
                    theme: *theme,
                    on_open: make_command_handler(DockCommand::OpenFlyout {
                        column: column_rect.column,
                        panel: item.panel,
                    }),
                }
                .into_any_element(),
            );
        }
    }

    elements
}


fn supported_splitter_handle(
    layout: &Layout,
    owner: NodeId,
    boundary_index: usize,
    axis: Axis,
) -> Option<ResizeHandle> {
    match axis {
        Axis::Y if matches!(layout.node(owner), Some(Node::Column(_))) => {
            Some(ResizeHandle::Splitter { owner, boundary_index, axis })
        }
        Axis::X
            if [DockSide::Left, DockSide::Right].into_iter().any(|side| {
                layout.main_area(side).is_some_and(|area| area.root == owner)
                    && !main_column_ids(layout, side).is_empty()
            }) =>
        {
            Some(ResizeHandle::Splitter { owner, boundary_index, axis })
        }
        _ => None,
    }
}


fn rect_element(rect: Rect, color: u32) -> Div {
    div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height))
        .bg(rgb(color))
}
