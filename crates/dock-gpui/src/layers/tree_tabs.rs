use dock_core::{
    ColumnMode, DockCommand, DragHandle, DragPayload, DropTarget, Layout, Node, NodeId, PanelId,
    Point, Rect, Size,
};
use gpui::{AnyElement, IntoElement};

use crate::components::focus_ring::{
    PanelMoveDirection, TabReorderDirection, keyboard_panel_move_direction,
    keyboard_tab_reorder_direction,
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
            adjacent_group_indices(source_index, column.groups.len());
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
    }))
}

fn adjacent_group_indices(
    source_index: usize,
    group_count: usize,
) -> (Option<usize>, Option<usize>) {
    if source_index >= group_count {
        return (None, None);
    }
    (
        source_index.checked_sub(1),
        (source_index + 1 < group_count).then_some(source_index + 1),
    )
}

#[cfg(test)]
mod keyboard_panel_move_tests {
    use super::adjacent_group_indices;

    #[test]
    fn panel_move_only_targets_adjacent_groups_within_column() {
        assert_eq!(adjacent_group_indices(0, 3), (None, Some(1)));
        assert_eq!(adjacent_group_indices(1, 3), (Some(0), Some(2)));
        assert_eq!(adjacent_group_indices(2, 3), (Some(1), None));
        assert_eq!(adjacent_group_indices(0, 1), (None, None));
        assert_eq!(adjacent_group_indices(3, 3), (None, None));
        assert_eq!(adjacent_group_indices(0, 0), (None, None));
    }
}

