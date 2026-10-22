use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, ColumnMode, DockCommand, DockSide, DragHandle, DragPayload, DragPreview, DropTarget,
    DropZoneRect, FloatingGroupId, Layout, LayoutRects, Node, NodeId, PanelId, PanelInstance, Point,
    Rect, ResizeEdge, Size, Tabs,
};
use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Div, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Render, ScrollHandle, StatefulInteractiveElement, Styled, Window,
    div, px, rgb,
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
        let ghost_title = format!("Move floating: {title}");
        container = container.child(
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
        let ghost_title = panel_title(instance);
        let content = rect_element(content_rect, theme.colors.group_background)
            .child(render_panel(instance))
            .id(format!("dock-flyout-panel-{:?}", flyout.panel))
            .on_mouse_down(
                MouseButton::Left,
                make_drag_start_handler(payload, DragHandle::FlyoutPanel),
            )
            .on_drag(payload, move |_payload, _offset, _window, app| {
                let title = ghost_title.clone();
                app.new(|_| DragGhost { title })
            });

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

/// Translates a Core drop zone by the GPUI scroll offset and clips it to the viewport.
fn scroll_rect_in_viewport(rect: Rect, viewport: Rect, scroll_offset_y: f32) -> Option<Rect> {
    if !scroll_offset_y.is_finite() {
        return None;
    }
    let top = (rect.origin.y + scroll_offset_y).max(viewport.origin.y);
    let bottom = (rect.origin.y + scroll_offset_y + rect.size.height)
        .min(viewport.origin.y + viewport.size.height);
    if !top.is_finite() || !bottom.is_finite() || bottom <= top {
        return None;
    }
    Some(Rect::new(
        Point::new(rect.origin.x, top),
        Size::new(rect.size.width, bottom - top),
    ))
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

        // Expanded groups have a header row above the tab strip. Collapsed groups
        // use their full reduced height for the strip and therefore skip this row.
        if !rail_groups.contains(&group.group)
            && let Some(Node::Tabs(tabs)) = layout.node(group.group)
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
                    .child(format!("▾ {title}"))
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
                    .focusable()
                    .focus_visible(|style| style.border_2().border_color(rgb(theme.colors.focus)))
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
        let ghost_title = format!("Move panel: {label}");
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
            .on_drag(payload, move |_payload, _offset, _window, app| {
                let title = ghost_title.clone();
                app.new(|_| DragGhost { title })
            })
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

fn translate_rect_to_local(rect: Rect, origin: Point) -> Rect {
    Rect::new(
        Point::new(rect.origin.x - origin.x, rect.origin.y - origin.y),
        rect.size,
    )
}

/// Produces tree geometry relative to a floating-container origin. Floating bounds, Canvas,
/// and drop-zone geometry stay in window coordinates and are intentionally left untouched.
fn localize_tree_rects(rects: &LayoutRects, origin: Point) -> LayoutRects {
    let mut local = rects.clone();
    for entry in &mut local.columns {
        entry.rect = translate_rect_to_local(entry.rect, origin);
    }
    for entry in &mut local.groups {
        entry.rect = translate_rect_to_local(entry.rect, origin);
    }
    for entry in &mut local.tab_strips {
        entry.rect = translate_rect_to_local(entry.rect, origin);
    }
    for entry in &mut local.tab_headers {
        entry.rect = translate_rect_to_local(entry.rect, origin);
    }
    for entry in &mut local.tab_overflows {
        entry.button_rect = translate_rect_to_local(entry.button_rect, origin);
    }
    for entry in &mut local.panels {
        entry.rect = translate_rect_to_local(entry.rect, origin);
    }
    for entry in &mut local.splitters {
        entry.rect = translate_rect_to_local(entry.rect, origin);
    }
    for entry in &mut local.vertical_overflow {
        entry.viewport = translate_rect_to_local(entry.viewport, origin);
        entry.content_extent = translate_rect_to_local(entry.content_extent, origin);
    }
    local
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
    use super::{rail_entry_label, scroll_rect_in_viewport, tab_header_label};
    use dock_core::ColumnMode;

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
    fn scrolled_drop_zones_are_translated_and_clipped_to_the_column_viewport() {
        let viewport = super::Rect::new(
            super::Point::new(20.0, 10.0),
            super::Size::new(180.0, 100.0),
        );
        let zone = super::Rect::new(
            super::Point::new(20.0, 120.0),
            super::Size::new(180.0, 40.0),
        );
        assert_eq!(
            scroll_rect_in_viewport(zone, viewport, -30.0),
            Some(super::Rect::new(
                super::Point::new(20.0, 90.0),
                super::Size::new(180.0, 20.0),
            ))
        );
        assert_eq!(scroll_rect_in_viewport(zone, viewport, f32::NAN), None);
    }

    #[test]
    fn floating_descendant_rects_are_relative_to_the_container_origin() {
        let rect = super::Rect::new(
            super::Point::new(145.0, 92.0),
            super::Size::new(180.0, 48.0),
        );
        assert_eq!(
            super::translate_rect_to_local(rect, super::Point::new(100.0, 60.0)),
            super::Rect::new(
                super::Point::new(45.0, 32.0),
                super::Size::new(180.0, 48.0),
            ),
        );
    }
}
