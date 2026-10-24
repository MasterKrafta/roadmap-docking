use std::collections::HashSet;

use dock_core::{Layout, LayoutRects, Node, NodeId, Point, Rect, Size};
use gpui::AnyElement;

use crate::components::{GroupFrame, GroupGrip, RectBox, TabStrip};
use crate::host::DockHost;
use crate::surface::RenderData;
use crate::tree_query::subtree_nodes;

/// Geometry and node subset for one tree paint operation.
#[derive(Clone, Copy)]
pub(crate) struct TreeArea<'a> {
    pub(super) rects: &'a LayoutRects,
    pub(super) nodes: &'a HashSet<NodeId>,
    pub(super) origin: Point,
    pub(super) content_column: Option<(NodeId, Rect)>,
}

impl TreeArea<'_> {
    pub(super) fn rect(&self, rect: Rect) -> Rect {
        crate::geometry::translate_rect_to_local(rect, self.origin)
    }

    pub(super) fn column_rect(&self, column: NodeId, rect: Rect) -> Rect {
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

/// Paints geometry-only surfaces, then delegates controls, panel bodies and rail entries
/// to small sublayers. Rectangles are translated individually, without cloning LayoutRects.
pub(crate) fn elements(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    area: TreeArea<'_>,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let theme = data.theme;
    let rects = area.rects;
    let nodes = area.nodes;
    let mut elements = Vec::new();

    let mut computed_rail_groups = HashSet::new();
    if data.frame.is_none() {
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
    let rail_groups = data.frame
        .map(crate::frame::FrameGeometry::rail_groups)
        .unwrap_or(&computed_rail_groups);

    for column in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        elements.push(
            RectBox::new(area.column_rect(column.column, column.rect), theme.colors.dock_background)
                .into_any_element(),
        );
    }

    for group in rects.groups.iter().filter(|entry| nodes.contains(&entry.group)) {
        let group_bounds = area.rect(group.rect);
        elements.push(
            GroupFrame { rect: group_bounds, theme: *theme }.into_any_element(),
        );

        if let Some(Node::Tabs(tabs)) = layout.node(group.group)
            && !tabs.collapsed
            && let Some(strip) = rects.tab_strips.iter().find(|strip| strip.group == group.group)
        {
            let grip_height = crate::chrome_geometry::GROUP_DRAG_GRIP_HEIGHT
                .min(theme.metrics.column_header_height)
                .min(group.rect.size.height);
            let strip_offset = strip.rect.origin.y - group.rect.origin.y;
            if grip_height > 0.0 && (strip_offset - grip_height).abs() < 0.01 {
                elements.push(
                    GroupGrip {
                        group: group.group,
                        rect: Rect::new(group_bounds.origin, Size::new(group.rect.size.width, grip_height)),
                        theme: *theme,
                        on_drag_start: host.drag_start(
                            dock_core::DragPayload::Group { group: group.group },
                            dock_core::DragHandle::GroupGrip,
                        ),
                    }.into_any_element(),
                );
            }
        }
    }

    for strip in rects.tab_strips.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            TabStrip { rect: area.rect(strip.rect), theme: *theme }.into_any_element(),
        );
    }

    elements.extend(super::tree_tabs::render(data, host, area, rail_groups));
    elements.extend(super::tree_panels::render(data, host, area));
    elements.extend(super::tree_rail::render(data, host, area));
    elements
}
