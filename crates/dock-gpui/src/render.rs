use std::collections::{HashMap, HashSet};

use crate::color::blend_rgb;
use crate::geometry::{localize_tree_rects, scroll_rect_in_viewport, translate_rect_to_local};

use dock_core::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DragHandle, DragPayload, DragPreview,
    DropTarget, DropZoneRect, FloatingGroupId, Layout, LayoutRects, Node, NodeId, PanelId,
    PanelInstance, Point, Rect, ResizeEdge, Size, Tabs,
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

const GROUP_MENU_BUTTON_WIDTH: f32 = 28.0;
const GROUP_DRAG_GRIP_HEIGHT: f32 = 4.0;

#[derive(Clone, Copy)]
struct GroupChromeGeometry {
    vertical_shift: f32,
}

/// Adapts Core's generic header-plus-tabs geometry to Photoshop's tabs-only
/// anatomy. The title-bar strip that belongs to a floating window is preserved;
/// any additional Tabs group header is removed. The same adjusted snapshot is
/// used by rendering and drop resolution so their hit targets remain aligned.
///
/// Call exactly once on a fresh LayoutRects snapshot.
pub fn normalize_docked_group_geometry(
    layout: &Layout,
    metrics: &DockMetrics,
    rects: &mut LayoutRects,
) {
    let mut rail_groups = HashSet::<NodeId>::new();
    for column_rect in &rects.columns {
        let Some(Node::Column(column)) = layout.node(column_rect.column) else {
            continue;
        };
        if column.mode.is_rail() {
            rail_groups.extend(column.groups.iter().copied());
        }
    }

    // A floating title is shared by every branch along the top edge of a
    // horizontal composition, not just the single group returned by min_by(y).
    // Core reserves title space for collapsed branches in composite roots;
    // standalone Tabs roots retain the adapter's simple local offset.
    let mut floating_title_groups = HashMap::<NodeId, (f32, f32)>::new();
    for (floating_id, floating) in layout.floating_groups_in_paint_order() {
        let Some(area) = layout.dock_area(floating.area) else {
            continue;
        };
        let Some(bounds) = rects
            .floating_groups
            .iter()
            .find(|entry| entry.group == floating_id)
            .map(|entry| entry.rect)
        else {
            continue;
        };
        if bounds.size.height <= 0.0 {
            continue;
        }
        let title_height = metrics.header_height.min(bounds.size.height);
        for top_group in floating_title_groups_in_tree(layout, area.root) {
            if rects.groups.iter().any(|entry| entry.group == top_group) {
                floating_title_groups.insert(top_group, (title_height, bounds.origin.y));
            }
        }
    }

    let mut groups = HashMap::<NodeId, GroupChromeGeometry>::new();
    let mut active_panel_shifts = HashMap::<PanelId, f32>::new();
    let mut floating_collapsed_shifts = HashMap::<NodeId, f32>::new();
    for group_rect in &rects.groups {
        let group_id = group_rect.group;
        if rail_groups.contains(&group_id) {
            continue;
        }
        let Some(Node::Tabs(tabs)) = layout.node(group_id) else {
            continue;
        };
        if let Some(&(title_height, window_top)) = floating_title_groups.get(&group_id) {
            // Expanded groups already place the strip below the floating title
            // through Core's header geometry. A collapsed group in a composite
            // root may already be below that title because Core reserved the
            // space during stack allocation. Only offset geometry that still
            // starts inside the title row (the standalone Tabs-root case).
            if tabs.collapsed {
                let shift = (window_top + title_height - group_rect.rect.origin.y).max(0.0);
                if shift > 0.001 {
                    floating_collapsed_shifts.insert(group_id, shift);
                }
            }
            continue;
        }
        let grip_height = if tabs.collapsed {
            0.0
        } else {
            GROUP_DRAG_GRIP_HEIGHT.min(metrics.header_height)
        };
        let vertical_shift = if tabs.collapsed {
            0.0
        } else {
            (metrics.header_height - grip_height).max(0.0)
        };
        groups.insert(group_id, GroupChromeGeometry { vertical_shift });
        if vertical_shift > 0.0
            && let Some(panel) = active_panel(layout, tabs)
        {
            active_panel_shifts.insert(panel, vertical_shift);
        }
    }

    // Keep collapsed floating tabs below the independent window title row.
    // Shift the group rectangle and hit targets together so the visual surface
    // and the Core drop geometry continue to describe the same area.
    for group in &mut rects.groups {
        if let Some(shift) = floating_collapsed_shifts.get(&group.group) {
            group.rect.origin.y += *shift;
        }
    }

    // Shift normalized docked strips into their former header row. Grow active
    // content by the same amount to preserve its bottom edge. Floating collapsed
    // strips instead move down to avoid being covered by the title bar.
    for strip in &mut rects.tab_strips {
        if let Some(group) = groups.get(&strip.group) {
            strip.rect.origin.y -= group.vertical_shift;
        } else if let Some(shift) = floating_collapsed_shifts.get(&strip.group) {
            strip.rect.origin.y += *shift;
        }
    }
    for header in &mut rects.tab_headers {
        if let Some(group) = groups.get(&header.group) {
            header.rect.origin.y -= group.vertical_shift;
        } else if let Some(shift) = floating_collapsed_shifts.get(&header.group) {
            header.rect.origin.y += *shift;
        }
    }
    for overflow in &mut rects.tab_overflows {
        if let Some(group) = groups.get(&overflow.group) {
            overflow.button_rect.origin.y -= group.vertical_shift;
        } else if let Some(shift) = floating_collapsed_shifts.get(&overflow.group) {
            overflow.button_rect.origin.y += *shift;
        }
    }
    for panel in &mut rects.panels {
        if let Some(shift) = active_panel_shifts.get(&panel.panel) {
            panel.rect.origin.y -= *shift;
            panel.rect.size.height += *shift;
        }
    }

    // Reserve separate horizontal areas for the panel menu and group dragging.
    // This is applied to floating Tabs groups too, so their menu never covers a tab.
    for group_rect in &rects.groups {
        let group_id = group_rect.group;
        if rail_groups.contains(&group_id) || !matches!(layout.node(group_id), Some(Node::Tabs(_))) {
            continue;
        }
        let width = group_rect.rect.size.width.max(0.0);
        let menu_width = GROUP_MENU_BUTTON_WIDTH.min(width);
        let has_overflow = rects.tab_overflows.iter().any(|entry| entry.group == group_id);
        let overflow_width = if has_overflow {
            metrics
                .tab_overflow_button_width
                .min((width - menu_width).max(0.0))
        } else {
            0.0
        };
        let tabs_width = (width - menu_width - overflow_width).max(0.0);

        let mut header_indices = rects
            .tab_headers
            .iter()
            .enumerate()
            .filter_map(|(index, header)| (header.group == group_id).then_some(index))
            .collect::<Vec<_>>();
        header_indices.sort_by_key(|index| rects.tab_headers[*index].index);
        let visible_count = header_indices.len();
        if visible_count > 0 {
            for (visible_index, header_index) in header_indices.into_iter().enumerate() {
                let left = group_rect.rect.origin.x
                    + tabs_width * visible_index as f32 / visible_count as f32;
                let right = if visible_index + 1 == visible_count {
                    group_rect.rect.origin.x + tabs_width
                } else {
                    group_rect.rect.origin.x
                        + tabs_width * (visible_index + 1) as f32 / visible_count as f32
                };
                let header = &mut rects.tab_headers[header_index];
                header.rect.origin.x = left;
                header.rect.size.width = (right - left).max(0.0);
            }
        }

        if has_overflow
            && let Some(overflow) = rects
                .tab_overflows
                .iter_mut()
                .find(|entry| entry.group == group_id)
        {
            overflow.button_rect.origin.x = group_rect.rect.origin.x + tabs_width;
            overflow.button_rect.size.width = overflow_width;
        }
    }
}

/// Finds one top Tabs group per vertical branch along the floating root's
/// upper edge. A Column contributes its first group; a horizontal Split
/// contributes the first group from each child branch.
fn floating_title_groups_in_tree(layout: &Layout, root: NodeId) -> Vec<NodeId> {
    match layout.node(root) {
        Some(Node::Tabs(_)) => vec![root],
        Some(Node::Column(column)) => {
            column.groups.first().copied().into_iter().collect()
        }
        Some(Node::Split(split)) if split.axis == Axis::X => split
            .children
            .iter()
            .flat_map(|child| floating_title_groups_in_tree(layout, *child))
            .collect(),
        Some(Node::Split(split)) => split
            .children
            .first()
            .map(|child| floating_title_groups_in_tree(layout, *child))
            .unwrap_or_default(),
        Some(Node::Panel(_)) | None => Vec::new(),
    }
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

/// Renders the current Core model and its freshly-computed geometry as GPUI elements.
///
/// The adapter reads Layout and emits typed command/menu requests; it does not mutate or clone
/// Layout. The app supplies panel content factories and owns all menu state.
pub fn render<F, T, I, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
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
        root = root.child(rect_element(canvas, theme.colors.canvas_background).into_any_element());
    }

    // Main dock areas are painted first; floating areas are layered above them.
    // Core's overflow records decide which columns get an actual scroll viewport.
    let mut main_column_overflows = Vec::new();
    for side in [DockSide::Left, DockSide::Right] {
        let Some(area) = layout.main_area(side) else {
            continue;
        };
        let nodes = subtree_nodes(layout, area.root);
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
            let column_nodes = subtree_nodes(layout, overflow.owner);
            for node in &column_nodes {
                non_scrolling_nodes.remove(node);
            }
            main_column_overflows.push((*overflow, column_nodes));
        }

        if !non_scrolling_nodes.is_empty() {
            for element in tree_elements(
                layout,
                rects,
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
            let column_nodes = subtree_nodes(layout, overflow.owner);
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
            rect_element(handle_rect, theme.colors.splitter)
                .id(format!("dock-outer-column-resize-{:?}", column_id))
                .on_mouse_down(
                    MouseButton::Left,
                    make_resize_start_handler(ResizeHandle::OuterColumn { column: column_id, side }),
                )
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
        let nodes = subtree_nodes(layout, area.root);
        let mut container = rect_element(rect, theme.colors.dock_background)
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme.colors.border));

        for element in tree_elements(
            layout,
            &local_rects,
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
            rect_element(title_rect, theme.colors.toolbar_background)
                .text_color(rgb(theme.colors.primary_text))
                .child(title)
                .id(format!("dock-floating-title-{:?}", floating_id))
                .on_mouse_down(
                    MouseButton::Left,
                    make_drag_start_handler(payload, DragHandle::FloatingTitle),
                )
                .into_any_element(),
        );
        root = root.child(container.into_any_element());
    }

    // Floating resize hit zones come from Core geometry. Put corners above long edges.
    for handle in rects.floating_resize_edges.iter().rev() {
        root = root.child(
            rect_element(handle.rect, theme.colors.splitter)
                .id(format!("dock-floating-resize-{:?}-{:?}", handle.group, handle.edge))
                .on_mouse_down(
                    MouseButton::Left,
                    make_resize_start_handler(ResizeHandle::FloatingEdge {
                        group: handle.group,
                        edge: handle.edge,
                    }),
                )
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
        let header = rect_element(header_rect, theme.colors.toolbar_background)
            .text_color(rgb(theme.colors.primary_text))
            .child(panel_title(instance));

        let button_width = (flyout.rect.size.width / 2.0).min(28.0);
        let menu_rect = Rect::new(
            Point::new(flyout.rect.size.width - button_width * 2.0, 0.0),
            Size::new(button_width, header_height),
        );
        let menu_button = rect_element(menu_rect, theme.colors.toolbar_background)
            .text_color(rgb(theme.colors.primary_text))
            .child("⋯")
            .id(format!("dock-flyout-menu-{:?}-{:?}", flyout.group, flyout.panel))
            .focusable()
            .focus_visible(|style| style.border_2().border_color(rgb(theme.colors.focus)))
            .on_click(make_menu_handler(DockMenuRequest::PanelGroup {
                group: flyout.group,
                panel: Some(flyout.panel),
            }));

        let close_rect = Rect::new(
            Point::new(flyout.rect.size.width - button_width, 0.0),
            Size::new(button_width, header_height),
        );
        let close_button = rect_element(close_rect, theme.colors.toolbar_background)
            .text_color(rgb(theme.colors.primary_text))
            .child("×")
            .id(format!("dock-flyout-close-{:?}-{:?}", flyout.group, flyout.panel))
            .focusable()
            .focus_visible(|style| style.border_2().border_color(rgb(theme.colors.focus)))
            .on_click(make_command_handler(DockCommand::CloseFlyout));

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
            .child(menu_button.into_any_element())
            .child(close_button.into_any_element())
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
            let weight = if active { 0.38 } else { 0.13 };
            let fill = blend_rgb(theme.colors.drop_target, theme.colors.canvas_background, weight);
            root = root.child(
                rect_element(zone_rect, fill)
                    .border_1()
                    .border_color(rgb(if active {
                        theme.colors.focus
                    } else {
                        theme.colors.drop_target
                    }))
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

fn drop_target_node(target: DropTarget) -> Option<NodeId> {
    match target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => Some(group),
        DropTarget::ColumnSlot { column, .. } | DropTarget::RailSlot { column, .. } => {
            Some(column)
        }
        DropTarget::MainColumn { .. } | DropTarget::FreeArea => None,
    }
}

fn tree_elements<F, T, I, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
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
    let mut rail_groups = HashSet::new();
    for column_rect in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        if matches!(
            layout.node(column_rect.column),
            Some(Node::Column(column)) if column.mode.is_rail()
        ) && let Ok(items) = layout.column_display_order(column_rect.column)
        {
            rail_groups.extend(items.into_iter().map(|item| item.group));
        }
    }

    for column in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        elements.push(
            rect_element(column.rect, theme.colors.dock_background).into_any_element(),
        );
    }

    for group in rects.groups.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            rect_element(group.rect, theme.colors.group_background)
                .border_1()
                .border_color(rgb(theme.colors.border))
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
                    rect_element(grip_rect, theme.colors.toolbar_background)
                        .id(format!("dock-group-grip-{:?}", group.group))
                        .on_mouse_down(
                            MouseButton::Left,
                            make_drag_start_handler(payload, DragHandle::GroupGrip),
                        )
                        .into_any_element(),
                );
            }
        }
    }

    // The strip background is presentation only. Group dragging belongs to
    // the separate thin grip above it; menu and tab controls remain independent.
    for strip in rects.tab_strips.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            rect_element(strip.rect, theme.colors.toolbar_background).into_any_element(),
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
        let background = if active {
            theme.colors.active_tab
        } else {
            theme.colors.toolbar_background
        };
        let payload = DragPayload::Panel { panel: header.panel };
        let activate_handler = make_command_handler(DockCommand::ActivateTab {
            group: header.group,
            panel: header.panel,
        });
        let collapse_handler = make_command_handler(DockCommand::CollapseGroup {
            group: header.group,
            collapsed: !collapsed,
        });
        let element = rect_element(header.rect, background)
            .text_color(rgb(theme.colors.primary_text))
            .child(display_label)
            .id(format!("dock-tab-{:?}-{:?}", header.group, header.panel))
            .focusable()
            .focus_visible(|style| style.border_2().border_color(rgb(theme.colors.focus)))
            .on_mouse_down(
                MouseButton::Left,
                make_drag_start_handler(payload, DragHandle::PanelTab),
            )
            .on_click(move |event, window, app| {
                if event.click_count() >= 2 {
                    collapse_handler(event, window, app);
                } else {
                    activate_handler(event, window, app);
                }
            });
        elements.push(element.into_any_element());
    }

    for overflow in rects
        .tab_overflows
        .iter()
        .filter(|entry| nodes.contains(&entry.group))
    {
        elements.push(
            rect_element(overflow.button_rect, theme.colors.toolbar_background)
                .text_color(rgb(theme.colors.primary_text))
                .child("⋯")
                .id(format!("dock-tab-overflow-{:?}", overflow.group))
                .focusable()
                .focus_visible(|style| style.border_2().border_color(rgb(theme.colors.focus)))
                .on_click(make_menu_handler(DockMenuRequest::TabOverflow {
                    group: overflow.group,
                }))
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
            rect_element(menu_rect, theme.colors.toolbar_background)
                .border_1()
                .border_color(rgb(theme.colors.border))
                .text_color(rgb(theme.colors.primary_text))
                .child("☰")
                .id(format!("dock-group-menu-{:?}", strip.group))
                .focusable()
                .focus_visible(|style| style.border_2().border_color(rgb(theme.colors.focus)))
                .on_click(make_menu_handler(DockMenuRequest::PanelGroup {
                    group: strip.group,
                    panel: active_panel(layout, tabs),
                }))
                .into_any_element(),
        );
    }

    let panel_node_ids: HashMap<PanelId, NodeId> = layout
        .nodes()
        .filter_map(|(node_id, node)| match node {
            Node::Panel(panel) => Some((panel.panel_id, node_id)),
            _ => None,
        })
        .collect();

    // Core emits content rectangles only for visible active panels. Call the
    // app-owned factory only after confirming that a panel belongs to this area.
    for panel_rect in rects.panels.iter().filter(|entry| {
        panel_node_ids
            .get(&entry.panel)
            .is_some_and(|node_id| nodes.contains(node_id))
    }) {
        let Some(instance) = layout.panel_instance(panel_rect.panel) else {
            continue;
        };
        elements.push(
            rect_element(panel_rect.rect, theme.colors.group_background)
                .child(render_panel(instance))
                .into_any_element(),
        );
    }

    for splitter in rects
        .splitters
        .iter()
        .filter(|entry| nodes.contains(&entry.owner))
    {
        let base = rect_element(splitter.rect, theme.colors.splitter);
        let handle = supported_splitter_handle(
            layout,
            splitter.owner,
            splitter.boundary_index,
            splitter.axis,
        );
        if let Some(handle) = handle {
            elements.push(
                base.id(format!("dock-splitter-{:?}-{}", splitter.owner, splitter.boundary_index))
                    .on_mouse_down(MouseButton::Left, make_resize_start_handler(handle))
                    .into_any_element(),
            );
        } else {
            elements.push(base.into_any_element());
        }
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
            let display_label = rail_entry_label(column.mode, icon, &label);
            let active_in_model = matches!(
                layout.node(item.group),
                Some(Node::Tabs(tabs)) if active_panel(layout, tabs) == Some(item.panel)
            );
            let active = selected_flyout.map_or(active_in_model, |panel| panel == item.panel);
            let background = if active {
                theme.colors.active_tab
            } else {
                theme.colors.toolbar_background
            };
            let entry = rect_element(item_rect, background)
                .text_color(rgb(theme.colors.primary_text));
            let entry = if column.mode == ColumnMode::IconsOnly {
                entry.flex().items_center().justify_center().child(display_label)
            } else {
                entry.flex().items_center().child(display_label)
            };
            elements.push(
                entry
                    .id(format!("dock-rail-{:?}-{:?}", column_rect.column, item.panel))
                    .focusable()
                    .focus_visible(|style| style.border_2().border_color(rgb(theme.colors.focus)))
                    .on_click(make_command_handler(DockCommand::OpenFlyout {
                        column: column_rect.column,
                        panel: item.panel,
                    }))
                    .into_any_element(),
            );
        }
    }

    elements
}

fn main_column_ids(layout: &Layout, side: DockSide) -> Vec<NodeId> {
    let Some(area) = layout.main_area(side) else {
        return Vec::new();
    };
    match layout.node(area.root) {
        Some(Node::Column(_)) => vec![area.root],
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children.iter().all(|child| matches!(layout.node(*child), Some(Node::Column(_)))) =>
        {
            split.children.clone()
        }
        _ => Vec::new(),
    }
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

fn first_panel_in_subtree(layout: &Layout, node_id: NodeId) -> Option<PanelId> {
    match layout.node(node_id)? {
        Node::Panel(panel) => Some(panel.panel_id),
        Node::Tabs(tabs) => active_panel(layout, tabs).or_else(|| {
            tabs.children
                .first()
                .and_then(|child| first_panel_in_subtree(layout, *child))
        }),
        Node::Column(column) => column
            .groups
            .first()
            .and_then(|group| first_panel_in_subtree(layout, *group)),
        Node::Split(split) => split
            .children
            .first()
            .and_then(|child| first_panel_in_subtree(layout, *child)),
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



fn subtree_nodes(layout: &Layout, root: NodeId) -> HashSet<NodeId> {
    let mut result = HashSet::new();
    let mut pending = vec![root];

    while let Some(id) = pending.pop() {
        if !result.insert(id) {
            continue;
        }

        let Some(node) = layout.node(id) else {
            continue;
        };
        match node {
            Node::Split(split) => pending.extend(split.children.iter().copied()),
            Node::Tabs(tabs) => pending.extend(tabs.children.iter().copied()),
            Node::Column(column) => pending.extend(column.groups.iter().copied()),
            Node::Panel(_) => {}
        }
    }

    result
}

fn active_panel(layout: &Layout, tabs: &Tabs) -> Option<PanelId> {
    let node_id = tabs.active_child()?;
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => Some(panel.panel_id),
        _ => None,
    }
}

fn rail_entry_label(mode: ColumnMode, icon: &str, title: &str) -> String {
    match mode {
        ColumnMode::IconsOnly => icon.to_owned(),
        ColumnMode::IconsAndLabels => format!("{icon}  {title}"),
        ColumnMode::Expanded => title.to_owned(),
    }
}

fn tab_header_label(title: &str, collapsed: bool, tab_index: usize) -> String {
    if collapsed && tab_index == 0 {
        format!("▸ {title}")
    } else {
        title.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_docked_group_geometry, rail_entry_label, tab_header_label,
        GROUP_DRAG_GRIP_HEIGHT, GROUP_MENU_BUTTON_WIDTH,
    };
    use dock_core::{
        create_initial_layout, ColumnMode, DockMetrics, DockSide, Node, PanelConstraints, PanelKey,
        Point, Profile, Rect, Size,
    };

    #[test]
    fn rail_modes_have_distinct_icon_and_label_presentations() {
        assert_eq!(rail_entry_label(ColumnMode::IconsOnly, "▱", "Layers"), "▱");
        assert_eq!(
            rail_entry_label(ColumnMode::IconsAndLabels, "▱", "Layers"),
            "▱  Layers"
        );
    }

    #[test]
    fn collapsed_stack_marker_appears_only_on_the_first_tab() {
        assert_eq!(tab_header_label("Layers", true, 0), "▸ Layers");
        assert_eq!(tab_header_label("Color", true, 1), "Color");
        assert_eq!(tab_header_label("Layers", false, 0), "Layers");
    }

    #[test]
    fn collapsed_floating_root_tabs_are_positioned_below_the_window_title() {
        use dock_core::DockCommand;

        let constraints = PanelConstraints::new(
            Size::new(80.0, 80.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let metrics = DockMetrics {
            header_height: 32.0,
            tab_strip_height: 24.0,
            ..DockMetrics::default()
        };
        let mut layout =
            create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.node(root).unwrap() else {
            panic!("Photoshop main area should use a column");
        };
        let group = column.groups[0];
        layout
            .apply(
                DockCommand::Float {
                    node: group,
                    bounds: Rect::new(Point::new(40.0, 50.0), Size::new(320.0, 240.0)),
                },
                &metrics,
            )
            .unwrap();
        layout
            .apply(
                DockCommand::CollapseGroup {
                    group,
                    collapsed: true,
                },
                &metrics,
            )
            .unwrap();

        let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 700.0));
        let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
        normalize_docked_group_geometry(&layout, &metrics, &mut rects);

        let floating = rects.floating_groups.first().unwrap().rect;
        let group_rect = rects.groups.iter().find(|entry| entry.group == group).unwrap().rect;
        let strip = rects.tab_strips.iter().find(|entry| entry.group == group).unwrap().rect;
        let expected_y = floating.origin.y + metrics.header_height;
        assert!((group_rect.origin.y - expected_y).abs() < 0.001);
        assert!((strip.origin.y - expected_y).abs() < 0.001);
        for header in rects.tab_headers.iter().filter(|entry| entry.group == group) {
            assert!((header.rect.origin.y - expected_y).abs() < 0.001);
        }
    }

    #[test]
    fn docked_tabs_replace_header_row_and_reserve_distinct_menu_and_drag_space() {
        let constraints = PanelConstraints::new(
            Size::new(80.0, 80.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let metrics = DockMetrics {
            header_height: 32.0,
            tab_strip_height: 24.0,
            ..DockMetrics::default()
        };
        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
        let viewport = Rect::new(Point::new(0.0, 0.0), Size::new(1000.0, 700.0));
        let mut rects = layout.compute_rects(viewport, &metrics).unwrap();
        let before = rects.clone();

        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.node(column_id).unwrap() else {
            panic!("Photoshop main area should use a column");
        };
        let group_id = column.groups[0];
        let group_rect = before.groups.iter().find(|entry| entry.group == group_id).unwrap().rect;
        let before_strip = before.tab_strips.iter().find(|entry| entry.group == group_id).unwrap().rect;
        assert_eq!(before_strip.origin.y, group_rect.origin.y + metrics.header_height);
        let Node::Tabs(tabs) = layout.node(group_id).unwrap() else {
            panic!("column children should be tab groups");
        };
        let active_panel_id = super::active_panel(&layout, tabs).unwrap();
        let before_panel = before.panels.iter().find(|entry| entry.panel == active_panel_id).unwrap().rect;

        normalize_docked_group_geometry(&layout, &metrics, &mut rects);

        let after_strip = rects.tab_strips.iter().find(|entry| entry.group == group_id).unwrap().rect;
        assert_eq!(
            after_strip.origin.y,
            group_rect.origin.y + GROUP_DRAG_GRIP_HEIGHT
        );
        let after_panel = rects.panels.iter().find(|entry| entry.panel == active_panel_id).unwrap().rect;
        assert_eq!(
            after_panel.origin.y,
            group_rect.origin.y + GROUP_DRAG_GRIP_HEIGHT + metrics.tab_strip_height
        );
        let before_bottom = before_panel.origin.y + before_panel.size.height;
        let after_bottom = after_panel.origin.y + after_panel.size.height;
        assert!((before_bottom - after_bottom).abs() < 0.001);

        let headers = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == group_id)
            .collect::<Vec<_>>();
        let tab_right = headers
            .iter()
            .map(|header| header.rect.origin.x + header.rect.size.width)
            .fold(group_rect.origin.x, f32::max);
        let menu_left = group_rect.origin.x + group_rect.size.width - GROUP_MENU_BUTTON_WIDTH;
        assert!(
            (menu_left - tab_right).abs() < 0.001,
            "the menu button should sit directly after the tabs, without sharing their hit area"
        );
    }

}
