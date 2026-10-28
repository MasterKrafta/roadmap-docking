use crate::{dock::initial_float_bounds, frame::FrameGeometry};
use dock_core::{ColumnMode, DockCommand, DockMetrics, Layout, Node, NodeId, PanelId, Point};

use super::MenuItemModel;

pub(super) fn items(
    layout: &Layout,
    metrics: &DockMetrics,
    frame: &FrameGeometry,
    group: NodeId,
    panel: Option<PanelId>,
) -> Vec<MenuItemModel> {
    let mut items = Vec::new();

    if let Some(panel) = panel {
        items.push(item(
            "close-panel",
            "Закрыть вкладку",
            false,
            Some(DockCommand::ClosePanel { panel }),
        ));
    }
    items.push(item(
        "close-group",
        "Закрыть группу",
        false,
        Some(DockCommand::CloseGroup { group }),
    ));

    if let Some(Node::Tabs(tabs)) = layout.node(group) {
        items.push(item(
            "collapse-group",
            if tabs.collapsed {
                "Развернуть группу"
            } else {
                "Свернуть группу"
            },
            tabs.collapsed,
            Some(DockCommand::CollapseGroup {
                group,
                collapsed: !tabs.collapsed,
            }),
        ));
    }

    let auto_collapse = layout.flyout_auto_collapse_enabled();
    items.push(item(
        "flyout-auto-collapse",
        "Автосворачивание flyout",
        auto_collapse,
        Some(DockCommand::SetFlyoutAutoCollapse {
            enabled: !auto_collapse,
        }),
    ));

    let parent_column = layout.nodes().find_map(|(column_id, node)| match node {
        Node::Column(column) if column.groups.contains(&group) => {
            Some((column_id, column.mode, column.groups.len()))
        }
        _ => None,
    });
    if let Some((column, current_mode, group_count)) = parent_column {
        for (mode, label, id) in [
            (ColumnMode::Expanded, "Обычный режим", "mode-expanded"),
            (
                ColumnMode::IconsAndLabels,
                "Значки и подписи",
                "mode-icons-labels",
            ),
            (ColumnMode::IconsOnly, "Только значки", "mode-icons"),
        ] {
            items.push(item(
                id,
                label,
                mode == current_mode,
                Some(DockCommand::SetColumnMode { column, mode }),
            ));
        }

        if group_count > 1 {
            let available_height = frame
                .rects()
                .columns
                .iter()
                .find(|entry| entry.column == column)
                .map(|entry| {
                    (entry.rect.size.height - metrics.splitter_thickness * (group_count - 1) as f32)
                        .max(0.0)
                });
            items.push(item(
                "reset-column-ratios",
                "Выровнять высоту групп",
                false,
                available_height.map(|available_height| DockCommand::ResetColumnRatios {
                    column,
                    available_height,
                }),
            ));
        }
    }

    let float_command = frame
        .rects()
        .groups
        .iter()
        .find(|entry| entry.group == group)
        .and_then(|entry| {
            let source = entry.rect;
            let viewport = frame.rects().viewport?;
            let grab = Point::new(source.size.width * 0.5, source.size.height * 0.5);
            let pointer = Point::new(
                source.origin.x + grab.x,
                source.origin.y + grab.y + metrics.header_height,
            );
            initial_float_bounds(source, grab, pointer, viewport, metrics).ok()
        })
        .map(|bounds| {
            if parent_column.is_some_and(|(_, _, count)| count > 1) {
                DockCommand::FloatGroupAsColumn { group, bounds }
            } else {
                DockCommand::Float {
                    node: group,
                    bounds,
                }
            }
        });
    items.push(item(
        "float-group",
        "Отделить в плавающее окно",
        false,
        float_command,
    ));

    items
}

fn item(id: &str, label: &str, checked: bool, command: Option<DockCommand>) -> MenuItemModel {
    MenuItemModel {
        id: id.to_owned(),
        label: label.to_owned(),
        checked,
        enabled: command.is_some(),
        command,
    }
}
