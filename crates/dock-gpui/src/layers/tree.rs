use std::collections::HashSet;

use dock_core::{DockCommand, Layout, LayoutRects, Node, NodeId, Point, Rect, Size};
use gpui::{AnyElement, IntoElement};

use crate::components::focus_ring::{GroupMoveDirection, keyboard_group_move_direction};
use crate::components::{GroupFrame, GroupGrip, RectBox, TabStrip};
use crate::host::{CommandKeyHandler, DockHost};
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
        for column_rect in rects
            .columns
            .iter()
            .filter(|entry| nodes.contains(&entry.column))
        {
            if matches!(
                layout.node(column_rect.column),
                Some(Node::Column(column)) if column.mode.is_rail()
            ) && let Ok(items) = layout.column_display_order(column_rect.column)
            {
                computed_rail_groups.extend(items.into_iter().map(|item| item.group));
            }
        }
    }
    let rail_groups = data
        .frame
        .map(crate::frame::FrameGeometry::rail_groups)
        .unwrap_or(&computed_rail_groups);

    for column in rects
        .columns
        .iter()
        .filter(|entry| nodes.contains(&entry.column))
    {
        elements.push(
            RectBox::new(
                area.column_rect(column.column, column.rect),
                theme.colors.dock_background,
            )
            .into_any_element(),
        );
    }

    for group in rects
        .groups
        .iter()
        .filter(|entry| nodes.contains(&entry.group))
    {
        let group_bounds = area.rect(group.rect);
        elements.push(
            GroupFrame {
                rect: group_bounds,
                theme: *theme,
            }
            .into_any_element(),
        );

        if let Some(Node::Tabs(tabs)) = layout.node(group.group)
            && !tabs.collapsed
            && let Some(strip) = rects
                .tab_strips
                .iter()
                .find(|strip| strip.group == group.group)
        {
            let grip_height = crate::chrome_geometry::GROUP_DRAG_GRIP_HEIGHT
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
                            dock_core::DragPayload::Group { group: group.group },
                            dock_core::DragHandle::GroupGrip,
                        ),
                        on_move_key: group_move_key_handler(layout, host, group.group),
                    }
                    .into_any_element(),
                );
            }
        }
    }

    for strip in rects
        .tab_strips
        .iter()
        .filter(|entry| nodes.contains(&entry.group))
    {
        elements.push(
            TabStrip {
                rect: area.rect(strip.rect),
                theme: *theme,
            }
            .into_any_element(),
        );
    }

    elements.extend(super::tree_tabs::render(data, host, area, rail_groups));
    elements.extend(super::tree_panels::render(data, host, area));
    elements.extend(super::tree_rail::render(data, host, area));
    elements
}

fn group_move_key_handler(
    layout: &Layout,
    host: &dyn DockHost,
    group: NodeId,
) -> Option<CommandKeyHandler> {
    let (column_id, index, group_count) = layout.nodes().find_map(|(column_id, node)| {
        let Node::Column(column) = node else {
            return None;
        };
        if column.mode.is_rail() {
            return None;
        }
        column
            .groups
            .iter()
            .position(|candidate| *candidate == group)
            .map(|index| (column_id, index, column.groups.len()))
    })?;
    if group_count < 2 {
        return None;
    }

    let move_previous = group_move_target_index(index, group_count, GroupMoveDirection::Previous)
        .map(|target_index| {
            host.command_key(DockCommand::MoveGroup {
                group,
                column: column_id,
                index: target_index,
            })
        });
    let move_next =
        group_move_target_index(index, group_count, GroupMoveDirection::Next).map(|target_index| {
            host.command_key(DockCommand::MoveGroup {
                group,
                column: column_id,
                index: target_index,
            })
        });

    Some(Box::new(
        move |event, window, app| match keyboard_group_move_direction(&event.keystroke) {
            Some(GroupMoveDirection::Previous) => {
                if let Some(handler) = &move_previous {
                    handler(event, window, app);
                }
            }
            Some(GroupMoveDirection::Next) => {
                if let Some(handler) = &move_next {
                    handler(event, window, app);
                }
            }
            None => {}
        },
    ))
}

fn group_move_target_index(
    current_index: usize,
    group_count: usize,
    direction: GroupMoveDirection,
) -> Option<usize> {
    match direction {
        GroupMoveDirection::Previous if current_index > 0 && current_index < group_count => {
            Some(current_index - 1)
        }
        GroupMoveDirection::Next if current_index < group_count.saturating_sub(1) => {
            // MoveGroup takes an insertion slot before source removal; skip over
            // the next group's original slot so the moved group ends up after it.
            Some(current_index + 2)
        }
        _ => None,
    }
}

#[cfg(test)]
mod keyboard_group_move_tests {
    use super::group_move_target_index;
    use crate::components::focus_ring::GroupMoveDirection;

    #[test]
    fn group_move_insertion_slots_respect_column_boundaries() {
        assert_eq!(
            group_move_target_index(0, 3, GroupMoveDirection::Previous),
            None
        );
        assert_eq!(
            group_move_target_index(0, 3, GroupMoveDirection::Next),
            Some(2)
        );
        assert_eq!(
            group_move_target_index(1, 3, GroupMoveDirection::Previous),
            Some(0)
        );
        assert_eq!(
            group_move_target_index(1, 3, GroupMoveDirection::Next),
            Some(3)
        );
        assert_eq!(
            group_move_target_index(2, 3, GroupMoveDirection::Previous),
            Some(1)
        );
        assert_eq!(
            group_move_target_index(2, 3, GroupMoveDirection::Next),
            None
        );
        assert_eq!(
            group_move_target_index(0, 1, GroupMoveDirection::Previous),
            None
        );
        assert_eq!(
            group_move_target_index(0, 1, GroupMoveDirection::Next),
            None
        );
        assert_eq!(
            group_move_target_index(0, 0, GroupMoveDirection::Previous),
            None
        );
    }
}
