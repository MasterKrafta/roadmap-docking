use std::collections::{HashMap, HashSet};

use dock_core::{
    DockCommand, DockSide, Layout, LayoutRects, Node, NodeId, PanelId, PanelInstance, Rect,
    Size, Tabs,
};
use gpui::{
    AnyElement, App, ClickEvent, Div, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

/// A GPUI click handler created by the app for a Core command.
///
/// The adapter describes user intent as a DockCommand; it never mutates or clones Layout.
pub type CommandClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Renders the current Core model and its freshly-computed geometry as GPUI elements.
///
/// `render_panel` is called only for content rectangles emitted by Core. Core omits
/// inactive tab contents from those rectangles, so their UI trees are never constructed.
pub fn render<F, T, H>(
    layout: &Layout,
    rects: &LayoutRects,
    theme: &Theme,
    mut render_panel: F,
    panel_title: T,
    make_command_handler: H,
) -> Div
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    H: Fn(DockCommand) -> CommandClickHandler,
{
    let mut root = div()
        .relative()
        .w(px(rects.viewport.map_or(1.0, |value| value.size.width)))
        .h(px(rects.viewport.map_or(1.0, |value| value.size.height)))
        .bg(rgb(theme.colors.window_background));

    if let Some(canvas) = rects.canvas {
        root = root.child(
            rect_element(canvas, theme.colors.canvas_background).into_any_element(),
        );
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
        ) {
            root = root.child(element);
        }
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
        ) {
            root = root.child(element);
        }
    }

    root
}

fn tree_elements<F, T, H>(
    layout: &Layout,
    rects: &LayoutRects,
    nodes: &HashSet<NodeId>,
    theme: &Theme,
    render_panel: &mut F,
    panel_title: &T,
    make_command_handler: &H,
) -> Vec<AnyElement>
where
    F: FnMut(&PanelInstance) -> AnyElement,
    T: Fn(&PanelInstance) -> String,
    H: Fn(DockCommand) -> CommandClickHandler,
{
    let mut elements = Vec::new();

    for column in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        elements.push(
            rect_element(column.rect, theme.colors.dock_background)
                .into_any_element(),
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
                .map(|instance| panel_title(instance))
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
                    .child(title)
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
        let element = rect_element(header.rect, background)
            .text_color(rgb(theme.colors.primary_text))
            .child(label)
            .on_click(make_command_handler(DockCommand::ActivateTab {
                group: header.group,
                panel: header.panel,
            }));
        elements.push(element.into_any_element());
    }

    // The overflow menu itself belongs to the app. The adapter only draws Core's
    // button geometry and does not create or own menu state.
    for overflow in rects
        .tab_overflows
        .iter()
        .filter(|entry| nodes.contains(&entry.group))
    {
        elements.push(
            rect_element(overflow.button_rect, theme.colors.toolbar_background)
                .text_color(rgb(theme.colors.primary_text))
                .child("⋯")
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
        elements.push(
            rect_element(splitter.rect, theme.colors.splitter).into_any_element(),
        );
    }

    elements
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
