use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, ColumnMode, DockCommand, DockSide, DragHandle, DragPayload, DragPreview, DropZoneRect,
    FloatingGroupId, Layout, LayoutRects, Node, NodeId, PanelId, PanelInstance, Point, Rect,
    ResizeEdge, Size, Tabs,
};
use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Div, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px, rgb,
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

struct DragGhost {
    title: String,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(8.0))
            .bg(rgb(0x0C8CE9))
            .text_color(rgb(0xFFFFFF))
            .child(self.title.clone())
    }
}

/// Requests app-owned menu state without transferring ownership of the layout to the adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockMenuRequest {
    TabOverflow { group: NodeId },
    PanelGroup { group: NodeId, panel: Option<PanelId> },
}

/// Renders the current Core model and its freshly-computed geometry as GPUI elements.
///
/// The adapter reads Layout and emits typed command/menu requests; it does not mutate or clone
/// Layout. The app supplies panel content factories and owns all menu state.
pub fn render<F, T, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    theme: &Theme,
    mut render_panel: F,
    panel_title: T,
    make_command_handler: H,
    make_menu_handler: M,
    make_drag_start_handler: D,
    make_resize_start_handler: R,
    drag_overlay: Option<&DragOverlay>,
) -> Div
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
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
    for side in [DockSide::Left, DockSide::Right] {
        let Some(area) = layout.main_area(side) else {
            continue;
        };
        let nodes = subtree_nodes(layout, area.root);
        for element in tree_elements(
            layout,
            rects,
            &nodes,
            theme,
            &mut render_panel,
            &panel_title,
            &make_command_handler,
            &make_menu_handler,
            &make_drag_start_handler,
            &make_resize_start_handler,
        ) {
            root = root.child(element);
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

        root = root.child(
            rect_element(rect, theme.colors.dock_background)
                .border_1()
                .border_color(rgb(theme.colors.border))
                .into_any_element(),
        );

        let Some(area) = layout.dock_area(floating.area) else {
            continue;
        };
        let nodes = subtree_nodes(layout, area.root);
        for element in tree_elements(
            layout,
            rects,
            &nodes,
            theme,
            &mut render_panel,
            &panel_title,
            &make_command_handler,
            &make_menu_handler,
            &make_drag_start_handler,
            &make_resize_start_handler,
        ) {
            root = root.child(element);
        }

        let title = first_panel_in_subtree(layout, area.root)
            .and_then(|panel| layout.panel_instance(panel))
            .map(&panel_title)
            .unwrap_or_else(|| "Floating group".to_owned());
        let title_height = theme.metrics.column_header_height.min(rect.size.height);
        let title_rect = Rect::new(rect.origin, Size::new(rect.size.width, title_height));
        let payload = DragPayload::FloatingGroup { group: floating_id };
        let ghost_title = format!("Move floating: {title}");
        root = root.child(
            rect_element(title_rect, theme.colors.toolbar_background)
                .text_color(rgb(theme.colors.primary_text))
                .child(title)
                .id(format!("dock-floating-title-{:?}", floating_id))
                .on_mouse_down(
                    MouseButton::Left,
                    make_drag_start_handler(payload, DragHandle::FloatingTitle),
                )
                .on_drag(payload, move |_payload, _offset, _window, app| {
                    let title = ghost_title.clone();
                    app.new(|_| DragGhost { title })
                })
                .into_any_element(),
        );
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

    // Core owns flyout selection and geometry. Only the selected flyout panel's factory is called.
    if let Some(flyout) = rects.flyout
        && let Some(instance) = layout.panel_instance(flyout.panel)
    {
        root = root.child(
            rect_element(flyout.rect, theme.colors.dock_background)
                .border_1()
                .border_color(rgb(theme.colors.border))
                .into_any_element(),
        );

        let header_height = theme.metrics.column_header_height.min(flyout.rect.size.height);
        let header_rect = Rect::new(
            flyout.rect.origin,
            Size::new(flyout.rect.size.width, header_height),
        );
        root = root.child(
            rect_element(header_rect, theme.colors.toolbar_background)
                .text_color(rgb(theme.colors.primary_text))
                .child(panel_title(instance))
                .into_any_element(),
        );

        let button_width = (flyout.rect.size.width / 2.0).min(28.0);
        let menu_rect = Rect::new(
            Point::new(
                flyout.rect.origin.x + flyout.rect.size.width - button_width * 2.0,
                flyout.rect.origin.y,
            ),
            Size::new(button_width, header_height),
        );
        root = root.child(
            rect_element(menu_rect, theme.colors.toolbar_background)
                .text_color(rgb(theme.colors.primary_text))
                .child("⋯")
                .id(format!("dock-flyout-menu-{:?}-{:?}", flyout.group, flyout.panel))
                .on_click(make_menu_handler(DockMenuRequest::PanelGroup {
                    group: flyout.group,
                    panel: Some(flyout.panel),
                }))
                .into_any_element(),
        );

        let close_rect = Rect::new(
            Point::new(
                flyout.rect.origin.x + flyout.rect.size.width - button_width,
                flyout.rect.origin.y,
            ),
            Size::new(button_width, header_height),
        );
        root = root.child(
            rect_element(close_rect, theme.colors.toolbar_background)
                .text_color(rgb(theme.colors.primary_text))
                .child("×")
                .id(format!("dock-flyout-close-{:?}-{:?}", flyout.group, flyout.panel))
                .on_click(make_command_handler(DockCommand::CloseFlyout))
                .into_any_element(),
        );

        let content_rect = Rect::new(
            Point::new(flyout.rect.origin.x, flyout.rect.origin.y + header_height),
            Size::new(
                flyout.rect.size.width,
                (flyout.rect.size.height - header_height).max(0.0),
            ),
        );
        let payload = DragPayload::Panel { panel: flyout.panel };
        let ghost_title = panel_title(instance);
        root = root.child(
            rect_element(content_rect, theme.colors.group_background)
                .child(render_panel(instance))
                .id(format!("dock-flyout-panel-{:?}", flyout.panel))
                .on_mouse_down(
                    MouseButton::Left,
                    make_drag_start_handler(payload, DragHandle::FlyoutPanel),
                )
                .on_drag(payload, move |_payload, _offset, _window, app| {
                    let title = ghost_title.clone();
                    app.new(|_| DragGhost { title })
                })
                .into_any_element(),
        );
    }

    if let Some(overlay) = drag_overlay {
        for zone in &overlay.zones {
            let active = overlay.preview.target == Some(zone.target);
            let weight = if active { 0.38 } else { 0.13 };
            let fill = blend_rgb(theme.colors.drop_target, theme.colors.canvas_background, weight);
            root = root.child(
                rect_element(zone.rect, fill)
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

fn tree_elements<F, T, H, M, D, R>(
    layout: &Layout,
    rects: &LayoutRects,
    nodes: &HashSet<NodeId>,
    theme: &Theme,
    render_panel: &mut F,
    panel_title: &T,
    make_command_handler: &H,
    make_menu_handler: &M,
    make_drag_start_handler: &D,
    make_resize_start_handler: &R,
) -> Vec<AnyElement>
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    H: Fn(DockCommand) -> CommandClickHandler,
    M: Fn(DockMenuRequest) -> CommandClickHandler,
    D: Fn(DragPayload, DragHandle) -> DragStartHandler,
    R: Fn(ResizeHandle) -> ResizeStartHandler,
{
    let mut elements = Vec::new();

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

        // Expanded groups have a header row above the tab strip. Collapsed groups
        // use their full reduced height for the strip and therefore skip this row.
        if let Some(Node::Tabs(tabs)) = layout.node(group.group)
            && !tabs.collapsed
        {
            let title = active_panel(layout, tabs)
                .and_then(|panel_id| layout.panel_instance(panel_id))
                .map(panel_title)
                .unwrap_or_else(|| "Panel group".to_owned());
            let header_rect = Rect::new(
                group.rect.origin,
                Size::new(
                    group.rect.size.width,
                    theme.metrics.column_header_height.min(group.rect.size.height),
                ),
            );
            elements.push(
                rect_element(header_rect, theme.colors.toolbar_background)
                    .text_color(rgb(theme.colors.primary_text))
                    .child(title.clone())
                    .into_any_element(),
            );

            // Separate group grip: menu and group drag have independent semantics.
            let button_width = header_rect.size.width.min(28.0);
            let grip_rect = Rect::new(
                Point::new(
                    header_rect.origin.x + header_rect.size.width - button_width * 2.0,
                    header_rect.origin.y,
                ),
                Size::new(button_width, header_rect.size.height),
            );
            let payload = DragPayload::Group { group: group.group };
            let ghost_title = format!("Move group: {title}");
            elements.push(
                rect_element(grip_rect, theme.colors.toolbar_background)
                    .text_color(rgb(theme.colors.primary_text))
                    .child("⠿")
                    .id(format!("dock-group-grip-{:?}", group.group))
                    .on_mouse_down(
                        MouseButton::Left,
                        make_drag_start_handler(payload, DragHandle::GroupGrip),
                    )
                    .on_drag(payload, move |_payload, _offset, _window, app| {
                        let title = ghost_title.clone();
                        app.new(|_| DragGhost { title })
                    })
                    .into_any_element(),
            );
            let menu_rect = Rect::new(
                Point::new(
                    header_rect.origin.x + header_rect.size.width - button_width,
                    header_rect.origin.y,
                ),
                Size::new(button_width, header_rect.size.height),
            );
            elements.push(
                rect_element(menu_rect, theme.colors.toolbar_background)
                    .text_color(rgb(theme.colors.primary_text))
                    .child("⋯")
                    .id(format!("dock-group-menu-{:?}", group.group))
                    .on_click(make_menu_handler(DockMenuRequest::PanelGroup {
                        group: group.group,
                        panel: active_panel(layout, tabs),
                    }))
                    .into_any_element(),
            );
        }
    }

    for strip in rects.tab_strips.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            rect_element(strip.rect, theme.colors.toolbar_background).into_any_element(),
        );
    }

    for header in rects
        .tab_headers
        .iter()
        .filter(|entry| nodes.contains(&entry.group))
    {
        let label = layout
            .panel_instance(header.panel)
            .map(panel_title)
            .unwrap_or_else(|| "Missing panel".to_owned());
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
        let ghost_title = format!("Move panel: {label}");
        let element = rect_element(header.rect, background)
            .text_color(rgb(theme.colors.primary_text))
            .child(label)
            .id(format!("dock-tab-{:?}-{:?}", header.group, header.panel))
            .on_mouse_down(
                MouseButton::Left,
                make_drag_start_handler(payload, DragHandle::PanelTab),
            )
            .on_drag(payload, move |_payload, _offset, _window, app| {
                let title = ghost_title.clone();
                app.new(|_| DragGhost { title })
            })
            .on_click(make_command_handler(DockCommand::ActivateTab {
                group: header.group,
                panel: header.panel,
            }));
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
                .on_click(make_menu_handler(DockMenuRequest::TabOverflow {
                    group: overflow.group,
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
        let element = if let Some(handle) = handle {
            base.id(format!("dock-splitter-{:?}-{}", splitter.owner, splitter.boundary_index))
                .on_mouse_down(MouseButton::Left, make_resize_start_handler(handle))
        } else {
            base
        };
        elements.push(element.into_any_element());
    }

    // Rail columns flatten their panel list for presentation while retaining the Core tree.
    let row_height = theme.metrics.tab_strip_height.max(28.0);
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

        for item in items {
            let y_offset = item.display_index as f32 * row_height;
            if y_offset >= column_rect.rect.size.height {
                break;
            }
            let item_height = row_height.min(column_rect.rect.size.height - y_offset);
            if item_height <= 0.0 {
                continue;
            }
            let item_rect = Rect::new(
                Point::new(column_rect.rect.origin.x, column_rect.rect.origin.y + y_offset),
                Size::new(column_rect.rect.size.width, item_height),
            );
            let label = layout
                .panel_instance(item.panel)
                .map(panel_title)
                .unwrap_or_else(|| "Missing panel".to_owned());
            let label = if column.mode == ColumnMode::IconsOnly {
                label.chars().next().unwrap_or('•').to_string()
            } else {
                label
            };
            elements.push(
                rect_element(item_rect, theme.colors.toolbar_background)
                    .text_color(rgb(theme.colors.primary_text))
                    .child(label)
                    .id(format!("dock-rail-{:?}-{:?}", column_rect.column, item.panel))
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

fn blend_rgb(foreground: u32, background: u32, foreground_weight: f32) -> u32 {
    let weight = foreground_weight.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let foreground = ((foreground >> shift) & 0xFF) as f32;
        let background = ((background >> shift) & 0xFF) as f32;
        (foreground * weight + background * (1.0 - weight))
            .round()
            .clamp(0.0, 255.0) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
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
