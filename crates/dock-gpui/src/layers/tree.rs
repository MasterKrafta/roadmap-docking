use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, DockCommand, DockSide, DragHandle, DragPayload, Layout, LayoutRects, Node, NodeId,
    PanelId, Point, Rect, Size,
};
use gpui::AnyElement;

use crate::chrome_geometry::{GROUP_DRAG_GRIP_HEIGHT, GROUP_MENU_BUTTON_WIDTH};
use crate::components::{
    GroupFrame, GroupGrip, GroupMenuButton, PanelSurface, PanelTab, RailEntry, RectBox,
    SplitterHandle, TabOverflowButton, TabStrip,
};
use crate::geometry::translate_rect_to_local;
use crate::host::DockHost;
use crate::labels::tab_header_label;
use crate::requests::{DockMenuRequest, ResizeHandle};
use crate::surface::RenderData;
use crate::tree_query::{active_panel, main_column_ids, subtree_nodes};

/// Geometry and node subset for one portion of a tree being painted.
///
/// `content_column` overrides one column rectangle for an overflowing scroll viewport;
/// all other rectangles are localized on demand without cloning the full `LayoutRects`.
pub(crate) struct TreeArea<'a> {
    pub rects: &'a LayoutRects,
    pub nodes: &'a HashSet<NodeId>,
    pub origin: Point,
    pub content_column: Option<(NodeId, Rect)>,
}

impl TreeArea<'_> {
    fn rect(&self, rect: Rect) -> Rect {
        translate_rect_to_local(rect, self.origin)
    }

    fn column_rect(&self, column: NodeId, rect: Rect) -> Rect {
        match self.content_column {
            Some((owner, bounds)) if owner == column => bounds,
            _ => self.rect(rect),
        }
    }
}

pub(crate) fn nodes_for_root(
    layout: &Layout,
    root: NodeId,
    frame: Option<&crate::frame::FrameGeometry>,
) -> HashSet<NodeId> {
    frame
        .and_then(|frame| frame.nodes_in_root(root).cloned())
        .unwrap_or_else(|| subtree_nodes(layout, root))
}

pub(crate) fn elements(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    area: TreeArea<'_>,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let frame = data.frame;
    let theme = data.theme;
    let rects = area.rects;
    let nodes = area.nodes;

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
        .map(crate::frame::FrameGeometry::rail_groups)
        .unwrap_or(&computed_rail_groups);

    for column in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        elements.push(
            RectBox::new(
                area.column_rect(column.column, column.rect),
                theme.colors.dock_background,
            )
            .into_any_element(),
        );
    }

    for group in rects.groups.iter().filter(|entry| nodes.contains(&entry.group)) {
        let group_bounds = area.rect(group.rect);
        elements.push(
            GroupFrame {
                rect: group_bounds,
                theme: *theme,
            }
            .into_any_element(),
        );

        // A four-pixel grip occupies the former header row; a floating root
        // uses its own title bar instead.
        if let Some(Node::Tabs(tabs)) = layout.node(group.group)
            && !tabs.collapsed
            && let Some(strip) = rects.tab_strips.iter().find(|strip| strip.group == group.group)
        {
            let grip_height = GROUP_DRAG_GRIP_HEIGHT
                .min(theme.metrics.column_header_height)
                .min(group.rect.size.height);
            let strip_offset = strip.rect.origin.y - group.rect.origin.y;
            if grip_height > 0.0 && (strip_offset - grip_height).abs() < 0.01 {
                elements.push(
                    GroupGrip {
                        group: group.group,
                        rect: Rect::new(
                            group_bounds.origin,
                            Size::new(group.rect.size.width, grip_height),
                        ),
                        theme: *theme,
                        on_drag_start: host.drag_start(
                            DragPayload::Group { group: group.group },
                            DragHandle::GroupGrip,
                        ),
                    }
                    .into_any_element(),
                );
            }
        }
    }

    for strip in rects.tab_strips.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            TabStrip {
                rect: area.rect(strip.rect),
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
            .map(|instance| host.panel_title(instance).to_string())
            .unwrap_or_else(|| "Missing panel".to_owned());
        let display_label = tab_header_label(&label, collapsed, header.index);
        let active = matches!(
            layout.node(header.group),
            Some(Node::Tabs(tabs)) if active_panel(layout, tabs) == Some(header.panel)
        );
        elements.push(
            PanelTab {
                id: format!("dock-tab-{:?}-{:?}", header.group, header.panel),
                rect: area.rect(header.rect),
                label: display_label,
                active,
                theme: *theme,
                on_activate: host.command(DockCommand::ActivateTab {
                    group: header.group,
                    panel: header.panel,
                }),
                on_collapse_toggle: host.command(DockCommand::CollapseGroup {
                    group: header.group,
                    collapsed: !collapsed,
                }),
                on_drag_start: host.drag_start(
                    DragPayload::Panel { panel: header.panel },
                    DragHandle::PanelTab,
                ),
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
                rect: area.rect(overflow.button_rect),
                theme: *theme,
                on_open: host.menu(DockMenuRequest::TabOverflow {
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
        let strip_bounds = area.rect(strip.rect);
        let button_width = GROUP_MENU_BUTTON_WIDTH.min(strip.rect.size.width);
        let menu_rect = Rect::new(
            Point::new(
                strip_bounds.origin.x + strip_bounds.size.width - button_width,
                strip_bounds.origin.y,
            ),
            Size::new(button_width, strip_bounds.size.height),
        );
        elements.push(
            GroupMenuButton {
                group: strip.group,
                panel: active_panel(layout, tabs),
                rect: menu_rect,
                theme: *theme,
                on_open: host.menu(DockMenuRequest::PanelGroup {
                    group: strip.group,
                    panel: active_panel(layout, tabs),
                }),
            }
            .into_any_element(),
        );
    }

    // Resolve panel ownership from the per-frame index; only legacy rectangle
    // rendering needs to construct this map on demand.
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
                rect: area.rect(panel_rect.rect),
                theme: *theme,
                content: host.render_panel(instance),
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
                rect: area.rect(splitter.rect),
                axis: splitter.axis,
                theme: *theme,
                on_resize_start: handle.map(|handle| host.resize_start(handle)),
            }
            .into_any_element(),
        );
    }

    // Rail modes flatten the Core tab order for presentation and reuse the
    // same header geometry as expanded columns.
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
        let column_bounds = area.column_rect(column_rect.column, column_rect.rect);
        let visible_bottom = column_bounds.origin.y + column_bounds.size.height;

        for item in items {
            let Some(header) = rects
                .tab_headers
                .iter()
                .find(|header| header.group == item.group && header.panel == item.panel)
            else {
                continue;
            };
            let header_bounds = area.rect(header.rect);
            if header_bounds.origin.y >= visible_bottom {
                break;
            }
            let height = header_bounds
                .size
                .height
                .min(visible_bottom - header_bounds.origin.y);
            if height <= 0.0 {
                continue;
            }
            let Some(instance) = layout.panel_instance(item.panel) else {
                continue;
            };
            elements.push(
                RailEntry {
                    id: format!("dock-rail-{:?}-{:?}", column_rect.column, item.panel),
                    rect: Rect::new(
                        header_bounds.origin,
                        Size::new(header_bounds.size.width, height),
                    ),
                    mode: column.mode,
                    icon: host.panel_icon(instance).to_owned(),
                    title: host.panel_title(instance).to_string(),
                    active: selected_flyout.map_or_else(
                        || matches!(
                            layout.node(item.group),
                            Some(Node::Tabs(tabs))
                                if active_panel(layout, tabs) == Some(item.panel)
                        ),
                        |panel| panel == item.panel,
                    ),
                    theme: *theme,
                    on_open: host.command(DockCommand::OpenFlyout {
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
