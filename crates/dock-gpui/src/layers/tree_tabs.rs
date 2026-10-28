use dock_core::{
    Axis, ColumnMode, DockCommand, DockSide, DragHandle, DragPayload, DropTarget, Layout, Node,
    NodeId, PanelId, Point, Rect, Size,
};
use gpui::{AnyElement, IntoElement};

use crate::components::focus_ring::{
    PanelColumnMoveDirection, PanelMoveDirection, TabReorderDirection,
    keyboard_panel_column_move_direction, keyboard_panel_move_direction,
    keyboard_panel_other_side_move, keyboard_panel_rail_move, keyboard_tab_reorder_direction,
};
use crate::components::{ColumnModeButton, GroupMenuButton, PanelTab, TabOverflowButton};
use crate::dock::{activate_tab, collapse_group, cycle_column_mode};
use crate::host::{CommandKeyHandler, DockHost};
use crate::labels::tab_header_label;
use crate::requests::DockMenuRequest;
use crate::surface::RenderData;
use crate::tree_query::active_panel;

use super::tree::TreeArea;

pub(crate) fn render(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    area: TreeArea<'_>,
    rail_groups: &std::collections::HashSet<NodeId>,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let theme = data.theme;
    let rects = area.rects;
    let nodes = area.nodes;
    let mut elements = Vec::new();

    for header in rects.tab_headers.iter().filter(|entry| {
        nodes.contains(&entry.group) && !rail_groups.contains(&entry.group)
    }) {
        let collapsed = matches!(layout.node(header.group), Some(Node::Tabs(tabs)) if tabs.collapsed);
        let label = layout.panel_instance(header.panel)
            .map(|instance| host.panel_title(instance).to_string())
            .unwrap_or_else(|| "Missing panel".to_owned());
        let display_label = tab_header_label(&label, collapsed, header.index);
        let active = matches!(
            layout.node(header.group),
            Some(Node::Tabs(tabs)) if active_panel(layout, tabs) == Some(header.panel)
        );
        let tab_count = match layout.node(header.group) {
            Some(Node::Tabs(tabs)) => tabs.children.len(),
            _ => 0,
        };

        elements.push(
            PanelTab {
                id: format!("dock-tab-{:?}-{:?}", header.group, header.panel),
                rect: area.rect(header.rect),
                label: display_label,
                active,
                theme: *theme,
                on_activate: host.command(activate_tab(header.group, header.panel)),
                on_activate_key: host.command_key(activate_tab(header.group, header.panel)),
                on_reorder_key: reorder_tab_key_handler(
                    host,
                    header.group,
                    header.panel,
                    header.index,
                    tab_count,
                ),
                on_move_panel_key: move_panel_key_handler(
                    layout,
                    host,
                    header.group,
                    header.panel,
                ),
                on_collapse_toggle: host.command(collapse_group(header.group, !collapsed)),
                on_drag_start: host.drag_start(
                    DragPayload::Panel { panel: header.panel },
                    DragHandle::PanelTab,
                ),
            }.into_any_element(),
        );
    }

    for overflow in rects.tab_overflows.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            TabOverflowButton {
                group: overflow.group,
                rect: area.rect(overflow.button_rect),
                theme: *theme,
                on_open: host.menu(DockMenuRequest::TabOverflow { group: overflow.group }),
            }.into_any_element(),
        );
    }

    for strip in rects.tab_strips.iter().filter(|entry| {
        nodes.contains(&entry.group) && !rail_groups.contains(&entry.group)
    }) {
        let Some(Node::Tabs(tabs)) = layout.node(strip.group) else { continue; };
        let strip_bounds = area.rect(strip.rect);
        let button_width = theme.metrics.group_control_button_width.min(strip.rect.size.width);
        let mode_button_column = layout.nodes().find_map(|(column_id, node)| match node {
            Node::Column(column)
                if column.groups.first().copied() == Some(strip.group)
                    && column.mode == ColumnMode::Expanded =>
            {
                Some((column_id, column.mode))
            }
            _ => None,
        });
        let mode_button_width = if mode_button_column.is_some() {
            theme
                .metrics
                .group_control_button_width
                .min((strip.rect.size.width - button_width).max(0.0))
        } else {
            0.0
        };
        if let Some((column_id, mode)) = mode_button_column
            && mode_button_width > 0.0
        {
            elements.push(
                ColumnModeButton {
                    column: column_id,
                    mode,
                    rect: Rect::new(
                        Point::new(
                            strip_bounds.origin.x + strip_bounds.size.width
                                - button_width - mode_button_width,
                            strip_bounds.origin.y,
                        ),
                        Size::new(mode_button_width, strip_bounds.size.height),
                    ),
                    theme: *theme,
                    on_toggle: host.command(cycle_column_mode(column_id, mode)),
                }
                .into_any_element(),
            );
        }
        let menu_rect = Rect::new(
            Point::new(strip_bounds.origin.x + strip_bounds.size.width - button_width, strip_bounds.origin.y),
            Size::new(button_width, strip_bounds.size.height),
        );
        let panel = active_panel(layout, tabs);
        elements.push(
            GroupMenuButton {
                group: strip.group,
                panel,
                rect: menu_rect,
                theme: *theme,
                on_open: host.menu(DockMenuRequest::PanelGroup { group: strip.group, panel }),
            }.into_any_element(),
        );
    }

    elements
}

fn reorder_tab_key_handler(
    host: &dyn DockHost,
    group: NodeId,
    panel: PanelId,
    index: usize,
    tab_count: usize,
) -> CommandKeyHandler {
    let move_previous = if index > 0 {
        Some(host.command_key(DockCommand::ReorderTab {
            group,
            panel,
            to_index: index - 1,
        }))
    } else {
        None
    };
    let move_next = if index + 1 < tab_count {
        Some(host.command_key(DockCommand::ReorderTab {
            group,
            panel,
            to_index: index + 1,
        }))
    } else {
        None
    };

    Box::new(move |event, window, app| {
        match keyboard_tab_reorder_direction(&event.keystroke) {
            Some(TabReorderDirection::Previous) => {
                if let Some(handler) = &move_previous {
                    handler(event, window, app);
                }
            }
            Some(TabReorderDirection::Next) => {
                if let Some(handler) = &move_next {
                    handler(event, window, app);
                }
            }
            None => {}
        }
    })
}

fn move_panel_key_handler(
    layout: &Layout,
    host: &dyn DockHost,
    source_group: NodeId,
    panel: PanelId,
) -> Option<CommandKeyHandler> {
    let (previous_group, next_group) = layout.nodes().find_map(|(_, node)| {
        let Node::Column(column) = node else {
            return None;
        };
        if column.mode.is_rail() {
            return None;
        }
        let source_index = column.groups.iter().position(|group| *group == source_group)?;
        let (previous_index, next_index) =
            adjacent_indices(source_index, column.groups.len());
        Some((
            previous_index.and_then(|index| column.groups.get(index).copied()),
            next_index.and_then(|index| column.groups.get(index).copied()),
        ))
    })?;

    let move_to_group = |target_group: NodeId| {
        let Node::Tabs(tabs) = layout.node(target_group)? else {
            return None;
        };
        Some(host.command_key(DockCommand::MovePanel {
            panel,
            target: DropTarget::Tabs {
                group: target_group,
                index: tabs.children.len(),
            },
        }))
    };
    let move_previous = previous_group.and_then(move_to_group);
    let move_next = next_group.and_then(move_to_group);
    let (previous_column_group, next_column_group) =
        adjacent_main_column_groups(layout, source_group);
    let move_previous_column = previous_column_group.and_then(move_to_group);
    let move_next_column = next_column_group.and_then(move_to_group);
    let move_other_side = opposite_main_dock_group(layout, source_group).and_then(move_to_group);
    let move_to_rail = main_rail_target_for_group(layout, source_group).map(|(column, index)| {
        host.command_key(DockCommand::MovePanel {
            panel,
            target: DropTarget::RailSlot { column, index },
        })
    });

    Some(Box::new(move |event, window, app| {
        match keyboard_panel_move_direction(&event.keystroke) {
            Some(PanelMoveDirection::Previous) => {
                if let Some(handler) = &move_previous {
                    handler(event, window, app);
                }
            }
            Some(PanelMoveDirection::Next) => {
                if let Some(handler) = &move_next {
                    handler(event, window, app);
                }
            }
            None => {}
        }
        match keyboard_panel_column_move_direction(&event.keystroke) {
            Some(PanelColumnMoveDirection::Previous) => {
                if let Some(handler) = &move_previous_column {
                    handler(event, window, app);
                }
            }
            Some(PanelColumnMoveDirection::Next) => {
                if let Some(handler) = &move_next_column {
                    handler(event, window, app);
                }
            }
            None => {}
        }
        if keyboard_panel_other_side_move(&event.keystroke) {
            if let Some(handler) = &move_other_side {
                handler(event, window, app);
            }
        }
        if keyboard_panel_rail_move(&event.keystroke) {
            if let Some(handler) = &move_to_rail {
                handler(event, window, app);
            }
        }
    }))
}

/// Returns the adjacent main-dock tab groups on the same side as the source group.
/// The target column follows the source group's vertical index, clamped when the
/// destination column contains fewer groups. Rail and empty columns are not targets.
fn adjacent_main_column_groups(
    layout: &Layout,
    source_group: NodeId,
) -> (Option<NodeId>, Option<NodeId>) {
    let source = layout.nodes().find_map(|(column_id, node)| match node {
        Node::Column(column) if !column.mode.is_rail() => column
            .groups
            .iter()
            .position(|group| *group == source_group)
            .map(|group_index| (column_id, group_index)),
        _ => None,
    });
    let Some((source_column, source_group_index)) = source else {
        return (None, None);
    };

    for side in [DockSide::Left, DockSide::Right] {
        let Some(columns) = main_column_ids(layout, side) else {
            continue;
        };
        let Some(source_column_index) = columns.iter().position(|id| *id == source_column) else {
            continue;
        };
        let (previous_index, next_index) = adjacent_indices(source_column_index, columns.len());

        let group_at = |column_index: Option<usize>| {
            let column_id = *columns.get(column_index?)?;
            let Node::Column(column) = layout.node(column_id)? else {
                return None;
            };
            if column.mode.is_rail() {
                return None;
            }
            let group_index = corresponding_group_index(source_group_index, column.groups.len())?;
            let group = column.groups.get(group_index).copied()?;
            matches!(layout.node(group), Some(Node::Tabs(_))).then_some(group)
        };

        return (group_at(previous_index), group_at(next_index));
    }

    (None, None)
}

/// Returns the ordered main columns for a side, or None when that dock tree is unsupported.
fn main_column_ids(layout: &Layout, side: DockSide) -> Option<Vec<NodeId>> {
    let area = layout.main_area(side)?;
    match layout.node(area.root) {
        Some(Node::Column(_)) => Some(vec![area.root]),
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children.iter().all(|child| {
                    matches!(layout.node(*child), Some(Node::Column(_)))
                }) =>
        {
            Some(split.children.clone())
        }
        _ => None,
    }
}

fn opposite_main_dock_group(layout: &Layout, source_group: NodeId) -> Option<NodeId> {
    let source = [DockSide::Left, DockSide::Right]
        .into_iter()
        .find_map(|side| {
            let columns = main_column_ids(layout, side)?;
            columns.iter().enumerate().find_map(|(column_index, column_id)| {
                let Node::Column(column) = layout.node(*column_id)? else {
                    return None;
                };
                if column.mode.is_rail() {
                    return None;
                }
                let group_index = column.groups.iter().position(|group| *group == source_group)?;
                Some((side, column_index, group_index))
            })
        })?;
    let target_side = match source.0 {
        DockSide::Left => DockSide::Right,
        DockSide::Right => DockSide::Left,
    };
    let target_columns = main_column_ids(layout, target_side)?;
    let target_column_index = corresponding_group_index(source.1, target_columns.len())?;
    let target_column_id = *target_columns.get(target_column_index)?;
    let Node::Column(target_column) = layout.node(target_column_id)? else {
        return None;
    };
    if target_column.mode.is_rail() {
        return None;
    }
    let target_group_index = corresponding_group_index(source.2, target_column.groups.len())?;
    let target_group = *target_column.groups.get(target_group_index)?;
    matches!(layout.node(target_group), Some(Node::Tabs(_))).then_some(target_group)
}

/// Returns the append slot of the first icon-rail column on the side owning this main-dock group.
fn main_rail_target_for_group(layout: &Layout, source_group: NodeId) -> Option<(NodeId, usize)> {
    let columns = [DockSide::Left, DockSide::Right].into_iter().find_map(|side| {
        let columns = main_column_ids(layout, side)?;
        let source_is_expanded = columns.iter().any(|column_id| {
            matches!(
                layout.node(*column_id),
                Some(Node::Column(column))
                    if !column.mode.is_rail() && column.groups.contains(&source_group)
            )
        });
        source_is_expanded.then_some(columns)
    })?;

    let rail_column = columns.into_iter().find(|column_id| {
        matches!(layout.node(*column_id), Some(Node::Column(column)) if column.mode.is_rail())
    })?;
    let index = layout.column_display_order(rail_column).ok()?.len();
    Some((rail_column, index))
}

fn corresponding_group_index(source_index: usize, target_group_count: usize) -> Option<usize> {
    if target_group_count == 0 {
        None
    } else {
        Some(source_index.min(target_group_count - 1))
    }
}

fn adjacent_indices(
    source_index: usize,
    item_count: usize,
) -> (Option<usize>, Option<usize>) {
    if source_index >= item_count {
        return (None, None);
    }
    (
        source_index.checked_sub(1),
        (source_index + 1 < item_count).then_some(source_index + 1),
    )
}

#[cfg(test)]
mod keyboard_panel_move_tests {
    use super::{
        adjacent_indices, corresponding_group_index, main_column_ids, main_rail_target_for_group,
        opposite_main_dock_group,
    };
    use crate::dock::DockController;
    use dock_core::{ColumnMode, DockCommand, DockSide, DropTarget, Node};
    use ui::Theme;

    #[test]
    fn panel_move_targets_append_slot_in_same_side_icon_rail() {
        let mut controller = DockController::new_initial(&Theme::default())
            .expect("initial layout should be valid");
        let initial_columns = main_column_ids(controller.layout(), DockSide::Right)
            .expect("initial layout should have a right dock");
        let initial_group = match controller.layout().node(initial_columns[0]) {
            Some(Node::Column(column)) => *column.groups.first().expect("column should have a group"),
            _ => panic!("main dock root should be a column"),
        };
        assert!(main_rail_target_for_group(controller.layout(), initial_group).is_none());

        controller
            .apply(DockCommand::MoveGroupToMainColumn {
                group: initial_group,
                side: DockSide::Right,
                index: 1,
            })
            .expect("splitting the group into a second main column should work");
        let columns = main_column_ids(controller.layout(), DockSide::Right)
            .expect("right dock should support ordered columns");
        assert_eq!(columns.len(), 2);

        let rail_column = columns[0];
        controller
            .apply(DockCommand::SetColumnMode {
                column: rail_column,
                mode: ColumnMode::IconsOnly,
            })
            .expect("first column should become an icon rail");

        let source_column = columns
            .iter()
            .copied()
            .find(|column_id| {
                *column_id != rail_column
                    && matches!(
                        controller.layout().node(*column_id),
                        Some(Node::Column(column)) if !column.mode.is_rail()
                    )
            })
            .expect("right dock should retain an expanded source column");
        let source_group = match controller.layout().node(source_column) {
            Some(Node::Column(column)) => *column.groups.first().expect("source column has a group"),
            _ => panic!("source column should be a column"),
        };
        let source_panel = match controller.layout().node(source_group) {
            Some(Node::Tabs(tabs)) => match controller.layout().node(
                *tabs.children.first().expect("source group should contain a panel"),
            ) {
                Some(Node::Panel(panel)) => panel.panel_id,
                _ => panic!("tab group child should be a panel"),
            },
            _ => panic!("source group should be a tab group"),
        };

        let (target_column, target_index) =
            main_rail_target_for_group(controller.layout(), source_group)
                .expect("expanded group should target the rail on its dock side");
        assert_eq!(target_column, rail_column);
        assert_eq!(
            target_index,
            controller
                .layout()
                .column_display_order(rail_column)
                .expect("rail display order should be available")
                .len()
        );

        controller
            .apply(DockCommand::MovePanel {
                panel: source_panel,
                target: DropTarget::RailSlot {
                    column: target_column,
                    index: target_index,
                },
            })
            .expect("moving a panel to the rail should work");
        assert!(
            controller
                .layout()
                .column_display_order(rail_column)
                .expect("rail display order should remain available")
                .iter()
                .any(|item| item.panel == source_panel)
        );
    }

    #[test]
    fn panel_move_only_targets_adjacent_items() {
        assert_eq!(adjacent_indices(0, 3), (None, Some(1)));
        assert_eq!(adjacent_indices(1, 3), (Some(0), Some(2)));
        assert_eq!(adjacent_indices(2, 3), (Some(1), None));
        assert_eq!(adjacent_indices(0, 1), (None, None));
        assert_eq!(adjacent_indices(3, 3), (None, None));
        assert_eq!(adjacent_indices(0, 0), (None, None));
    }

    #[test]
    fn column_move_preserves_vertical_group_position_when_possible() {
        assert_eq!(corresponding_group_index(0, 3), Some(0));
        assert_eq!(corresponding_group_index(1, 3), Some(1));
        assert_eq!(corresponding_group_index(2, 3), Some(2));
        assert_eq!(corresponding_group_index(2, 1), Some(0));
        assert_eq!(corresponding_group_index(4, 2), Some(1));
        assert_eq!(corresponding_group_index(0, 0), None);
    }

    #[test]
    fn panel_side_move_targets_a_regular_group_on_the_opposite_side() {
        let mut controller = DockController::new_initial(&Theme::default())
            .expect("initial layout should be valid");
        let group_to_move = main_column_ids(controller.layout(), DockSide::Right)
            .expect("initial layout should have a right dock")
            .into_iter()
            .find_map(|column_id| match controller.layout().node(column_id) {
                Some(Node::Column(column)) => column.groups.first().copied(),
                _ => None,
            })
            .expect("initial layout should have a right-side group");
        controller
            .apply(DockCommand::MoveGroupToMainColumn {
                group: group_to_move,
                side: DockSide::Left,
                index: 0,
            })
            .expect("moving a group should create the left dock");

        let layout = controller.layout();
        let source_group = main_column_ids(layout, DockSide::Left)
            .expect("moving the group should create a left dock")
            .into_iter()
            .find_map(|column_id| match layout.node(column_id) {
                Some(Node::Column(column)) if !column.mode.is_rail() => {
                    column.groups.first().copied()
                }
                _ => None,
            })
            .expect("left dock should contain a regular group");
        let target_group = opposite_main_dock_group(layout, source_group)
            .expect("opposite dock should contain a regular target group");
        let right_columns = main_column_ids(layout, DockSide::Right)
            .expect("right dock should remain available");

        assert!(right_columns.into_iter().any(|column_id| {
            matches!(
                layout.node(column_id),
                Some(Node::Column(column)) if column.groups.contains(&target_group)
            )
        }));
    }
}

