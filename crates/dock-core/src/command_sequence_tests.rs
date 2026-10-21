//! C4.7 regression coverage for command failures and generated command sequences.
//!
//! Commands are assembled from the current Layout views rather than retaining
//! IDs across mutations. This exercises the transactional API as a UI adapter
//! would use it.

use std::collections::HashSet;

use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ChromeMode, ColumnMode, DockCommand, DockMetrics, DockSide,
    DropTarget, DropZone, Layout, Node, NodeId, PanelConstraints, PanelId,
    PanelKey, Point, Profile, Rect, Size,
};

fn constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(80.0, 60.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        false,
    )
}

fn descriptors() -> Vec<(PanelKey, PanelConstraints)> {
    ["layers", "color", "properties", "history"]
        .into_iter()
        .map(|name| (PanelKey::new(name).unwrap(), constraints()))
        .collect()
}

fn initial_layout(profile: Profile) -> Layout {
    crate::create_initial_layout(profile, &descriptors(), DockMetrics::default()).unwrap()
}

fn sequence_layout() -> Layout {
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let mut columns = Vec::new();

    for column_index in 0..2 {
        let mut groups = Vec::new();
        for group_index in 0..2 {
            let mut panels = Vec::new();
            for panel_index in 0..2 {
                let key = PanelKey::new(format!(
                    "sequence-panel-{column_index}-{group_index}-{panel_index}"
                ))
                .unwrap();
                let (_, panel) = builder.panel(key, constraints());
                panels.push(panel);
            }
            groups.push(builder.tabs(panels, 0, false));
        }
        columns.push(builder.column(
            280.0,
            ColumnMode::Expanded,
            groups,
            vec![0.5, 0.5],
        ));
    }

    let root = builder.split(Axis::X, columns, vec![0.5, 0.5]);
    builder.main_area(DockSide::Right, root);
    let layout = builder.build();
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    layout
}

fn panel_nodes(layout: &Layout) -> Vec<(NodeId, PanelId)> {
    layout
        .nodes()
        .filter_map(|(node_id, node)| match node {
            Node::Panel(panel) => Some((node_id, panel.panel_id)),
            _ => None,
        })
        .collect()
}

fn tab_groups(layout: &Layout) -> Vec<(NodeId, Vec<NodeId>, usize, bool)> {
    layout
        .nodes()
        .filter_map(|(node_id, node)| match node {
            Node::Tabs(tabs) => Some((
                node_id,
                tabs.children.clone(),
                tabs.active_index,
                tabs.collapsed,
            )),
            _ => None,
        })
        .collect()
}

type ColumnInfo = (NodeId, Vec<NodeId>, f32, ColumnMode, Vec<f32>);

fn collect_main_columns(layout: &Layout, root: NodeId, output: &mut Vec<ColumnInfo>) {
    match layout.node(root) {
        Some(Node::Column(column)) => output.push((
            root,
            column.groups.clone(),
            column.width,
            column.mode,
            column.group_ratios.clone(),
        )),
        Some(Node::Split(split)) if split.axis == Axis::X => {
            for child in &split.children {
                collect_main_columns(layout, *child, output);
            }
        }
        _ => {}
    }
}

fn main_columns(layout: &Layout, side: DockSide) -> Vec<ColumnInfo> {
    let mut columns = Vec::new();
    if let Some(area) = layout.main_area(side) {
        collect_main_columns(layout, area.root, &mut columns);
    }
    columns
}

fn nodes_in_subtree(layout: &Layout, root: NodeId) -> HashSet<NodeId> {
    let mut found = HashSet::new();
    let mut pending = vec![root];
    while let Some(node_id) = pending.pop() {
        if !found.insert(node_id) {
            continue;
        }
        match layout.node(node_id) {
            Some(Node::Split(split)) => pending.extend(split.children.iter().copied()),
            Some(Node::Tabs(tabs)) => pending.extend(tabs.children.iter().copied()),
            Some(Node::Column(column)) => pending.extend(column.groups.iter().copied()),
            Some(Node::Panel(_)) | None => {}
        }
    }
    found
}

fn main_tab_groups(layout: &Layout) -> Vec<(NodeId, Vec<NodeId>, usize, bool)> {
    let mut main_nodes = HashSet::new();
    for side in [DockSide::Left, DockSide::Right] {
        if let Some(area) = layout.main_area(side) {
            main_nodes.extend(nodes_in_subtree(layout, area.root));
        }
    }

    tab_groups(layout)
        .into_iter()
        .filter(|(group, _, _, _)| main_nodes.contains(group))
        .collect()
}

fn assert_rejected_atomically(
    layout: &mut Layout,
    command: DockCommand,
    metrics: &DockMetrics,
) {
    let description = format!("{command:?}");
    let before = format!("{layout:?}");
    let version = layout.version();

    let result = layout.apply(command, metrics);
    assert!(
        result.is_err(),
        "expected {description} to be rejected, got {result:?}"
    );
    assert_eq!(format!("{layout:?}"), before, "{description} mutated Layout");
    assert_eq!(layout.version(), version, "{description} changed version");
    assert_eq!(layout.validate(metrics), Ok(()), "rollback left invalid Layout");
}

#[test]
fn stale_handles_are_rejected_atomically_for_every_id_bearing_command() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Generic);
    let column = layout.main_area(DockSide::Right).unwrap().root;
    let group = tab_groups(&layout)[0].0;
    let panel = panel_nodes(&layout)[0].1;
    let target = DropTarget::Tabs { group, index: 0 };
    let bounds = Rect::new(Point::new(20.0, 30.0), Size::new(320.0, 220.0));

    // Every command below references a stale ID or an invalid insertion slot.
    // The helper compares the entire Layout debug state as well as version.
    let commands = vec![
        DockCommand::OpenPanel {
            key: PanelKey::new("stale-target-open").unwrap(),
            constraints: constraints(),
            target: Some(DropTarget::Tabs {
                group: NodeId::null(),
                index: 0,
            }),
        },
        DockCommand::ClosePanel {
            panel: PanelId::null(),
        },
        DockCommand::CloseGroup {
            group: NodeId::null(),
        },
        DockCommand::ActivateTab {
            group: NodeId::null(),
            panel,
        },
        DockCommand::ReorderTab {
            group: NodeId::null(),
            panel,
            to_index: 0,
        },
        DockCommand::MovePanel {
            panel: PanelId::null(),
            target,
        },
        DockCommand::MoveGroup {
            group: NodeId::null(),
            column,
            index: 0,
        },
        DockCommand::SplitGroup {
            group: NodeId::null(),
            target_group: group,
            zone: DropZone::Bottom,
            ratio: 0.5,
        },
        DockCommand::ResizeGroups {
            column: NodeId::null(),
            boundary_index: 0,
            delta: 1.0,
            available_height: 1200.0,
        },
        DockCommand::ResizeColumn {
            column: NodeId::null(),
            width: 300.0,
        },
        DockCommand::ResizeColumnBoundary {
            side: DockSide::Right,
            boundary_index: usize::MAX,
            delta: 1.0,
        },
        DockCommand::ResetColumnRatios {
            column: NodeId::null(),
            available_height: 1200.0,
        },
        DockCommand::RestoreGroupRatios {
            column: NodeId::null(),
            ratios: vec![],
        },
        DockCommand::Float {
            node: NodeId::null(),
            bounds,
        },
        DockCommand::MoveFloating {
            group: crate::FloatingGroupId::null(),
            origin: Point::new(60.0, 70.0),
        },
        DockCommand::ResizeFloating {
            group: crate::FloatingGroupId::null(),
            size: Size::new(360.0, 260.0),
        },
        DockCommand::RaiseFloating {
            group: crate::FloatingGroupId::null(),
        },
        DockCommand::DockFloating {
            group: crate::FloatingGroupId::null(),
            target,
        },
        DockCommand::CollapseGroup {
            group: NodeId::null(),
            collapsed: true,
        },
        DockCommand::SetColumnMode {
            column: NodeId::null(),
            mode: ColumnMode::IconsOnly,
        },
        DockCommand::OpenFlyout {
            column: NodeId::null(),
            group,
        },
    ];

    for command in commands {
        assert_rejected_atomically(&mut layout, command, &metrics);
    }

    // CloseFlyout, SetChrome and ResetMainColumnRatios have no runtime ID
    // argument. Their legal no-op paths are covered separately by command and
    // sequence tests instead of manufacturing a stale-handle failure.
}

#[test]
fn existing_but_wrong_node_types_and_forbidden_targets_are_rejected_atomically() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Photoshop);
    let column = layout.main_area(DockSide::Right).unwrap().root;
    let group = tab_groups(&layout)[0].0;
    let panel_node_and_id = panel_nodes(&layout)[0];
    let panel = panel_node_and_id.1;
    let panel_node = panel_node_and_id.0;

    let wrong_type_commands = vec![
        DockCommand::CloseGroup { group: column },
        DockCommand::ActivateTab { group: column, panel },
        DockCommand::ReorderTab {
            group: column,
            panel,
            to_index: 0,
        },
        DockCommand::MoveGroup {
            group,
            column: group,
            index: 0,
        },
        DockCommand::SplitGroup {
            group,
            target_group: column,
            zone: DropZone::Top,
            ratio: 0.5,
        },
        DockCommand::ResizeGroups {
            column: group,
            boundary_index: 0,
            delta: 2.0,
            available_height: 1200.0,
        },
        DockCommand::ResizeColumn {
            column: group,
            width: 300.0,
        },
        DockCommand::ResetColumnRatios {
            column: group,
            available_height: 1200.0,
        },
        DockCommand::RestoreGroupRatios {
            column: group,
            ratios: vec![1.0],
        },
        DockCommand::CollapseGroup {
            group: column,
            collapsed: true,
        },
        DockCommand::SetColumnMode {
            column: group,
            mode: ColumnMode::IconsOnly,
        },
        DockCommand::OpenFlyout { column, group },
        DockCommand::Float {
            node: column,
            bounds: Rect::new(Point::new(20.0, 30.0), Size::new(320.0, 220.0)),
        },
        DockCommand::MovePanel {
            panel,
            target: DropTarget::RailSlot { column, index: 0 },
        },
        DockCommand::OpenPanel {
            key: PanelKey::new("forbidden-rail-open").unwrap(),
            constraints: constraints(),
            target: Some(DropTarget::RailSlot { column, index: 0 }),
        },
    ];

    for command in wrong_type_commands {
        assert_rejected_atomically(&mut layout, command, &metrics);
    }

    // A flyout requires a rail column and a group owned by that column.
    layout
        .apply(
            DockCommand::SetColumnMode {
                column,
                mode: ColumnMode::IconsOnly,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.validate(&metrics), Ok(()));
    assert_rejected_atomically(
        &mut layout,
        DockCommand::OpenFlyout {
            column,
            group: panel_node,
        },
        &metrics,
    );

    // Docking a real floating window to a rail slot is still forbidden while
    // the target column is expanded.
    layout
        .apply(
            DockCommand::SetColumnMode {
                column,
                mode: ColumnMode::Expanded,
            },
            &metrics,
        )
        .unwrap();
    layout
        .apply(
            DockCommand::Float {
                node: panel_node,
                bounds: Rect::new(Point::new(40.0, 50.0), Size::new(320.0, 220.0)),
            },
            &metrics,
        )
        .unwrap();
    let floating = layout.floating_groups().next().unwrap().0;
    assert_rejected_atomically(
        &mut layout,
        DockCommand::DockFloating {
            group: floating,
            target: DropTarget::RailSlot { column, index: 0 },
        },
        &metrics,
    );
}

#[test]
fn source_equals_target_and_same_position_commands_are_successful_noops() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (panel_id, first_panel) =
        builder.panel(PanelKey::new("same-target-first").unwrap(), constraints());
    let (_, second_panel) =
        builder.panel(PanelKey::new("same-target-second").unwrap(), constraints());
    let group = builder.tabs(vec![first_panel, second_panel], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    assert_eq!(layout.validate(&metrics), Ok(()));
    let before = format!("{layout:?}");

    let noops = [
        DockCommand::MovePanel {
            panel: panel_id,
            target: DropTarget::Tabs { group, index: 0 },
        },
        DockCommand::MoveGroup {
            group,
            column,
            index: 0,
        },
        DockCommand::SplitGroup {
            group,
            target_group: group,
            zone: DropZone::Bottom,
            ratio: 0.5,
        },
        DockCommand::ReorderTab {
            group,
            panel: panel_id,
            to_index: 0,
        },
    ];

    for command in noops {
        let outcome = layout.apply(command, &metrics).unwrap();
        assert_eq!(outcome.new_version, 0);
        assert_eq!(outcome.inverse, None);
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }
}

fn valid_commands(
    layout: &Layout,
    metrics: &DockMetrics,
    serial: &mut usize,
) -> Vec<DockCommand> {
    let groups = tab_groups(layout);
    let main_groups = main_tab_groups(layout);
    let panels = panel_nodes(layout);
    let columns = [DockSide::Left, DockSide::Right]
        .into_iter()
        .flat_map(|side| main_columns(layout, side))
        .collect::<Vec<_>>();
    let right_columns = main_columns(layout, DockSide::Right);
    let mut commands = vec![
        DockCommand::CloseFlyout,
        DockCommand::SetChrome {
            mode: if layout.chrome() == ChromeMode::Visible {
                ChromeMode::Hidden
            } else {
                ChromeMode::Visible
            },
        },
        DockCommand::ResetMainColumnRatios {
            side: DockSide::Right,
        },
        DockCommand::ResetMainColumnRatios {
            side: DockSide::Left,
        },
    ];

    // OpenPanel is always generated with a target available in the current
    // layout, falling back to a valid new main-column slot when the dock is empty.
    let open_target = main_groups
        .first()
        .map(|(group, children, _, _)| DropTarget::Tabs {
            group: *group,
            index: children.len(),
        })
        .unwrap_or(DropTarget::MainColumn {
            side: DockSide::Right,
            index: 0,
        });
    let key = PanelKey::new(format!("sequence-added-{}", *serial)).unwrap();
    *serial += 1;
    commands.push(DockCommand::OpenPanel {
        key,
        constraints: constraints(),
        target: Some(open_target),
    });

    for (group, children, active_index, collapsed) in &groups {
        if children.len() > 1 {
            if let Some(panel_id) = children
                .get(*active_index)
                .and_then(|node_id| match layout.node(*node_id) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                })
            {
                commands.push(DockCommand::ReorderTab {
                    group: *group,
                    panel: panel_id,
                    to_index: children.len() - 1,
                });
            }
            if let Some(panel_id) = children.iter().enumerate().find_map(|(index, node_id)| {
                if index == *active_index {
                    return None;
                }
                match layout.node(*node_id) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                }
            }) {
                commands.push(DockCommand::ActivateTab {
                    group: *group,
                    panel: panel_id,
                });
            }
        }
        commands.push(DockCommand::CollapseGroup {
            group: *group,
            collapsed: !*collapsed,
        });
    }

    // Exercise ClosePanel / CloseGroup without allowing a generated command to
    // remove the last available panel or tab group from the generated layout.
    if panels.len() > 4 {
        if let Some((_, panel_id)) = panels.first() {
            commands.push(DockCommand::ClosePanel { panel: *panel_id });
        }
    }
    if groups.len() > 3 {
        commands.push(DockCommand::CloseGroup { group: groups[0].0 });
    }

    for (panel_node, panel_id) in &panels {
        let source_group = groups
            .iter()
            .find(|(_, children, _, _)| children.contains(panel_node))
            .map(|(group, _, _, _)| *group);
        for (target_group, children, _, _) in &groups {
            if Some(*target_group) != source_group {
                commands.push(DockCommand::MovePanel {
                    panel: *panel_id,
                    target: DropTarget::Tabs {
                        group: *target_group,
                        index: children.len(),
                    },
                });
            }
        }
    }

    for (_, source_groups, _, _, _) in &columns {
        for source_group in source_groups {
            for (target_column, target_groups, _, _, _) in &columns {
                for index in 0..=target_groups.len() {
                    commands.push(DockCommand::MoveGroup {
                        group: *source_group,
                        column: *target_column,
                        index,
                    });
                }
            }
        }
    }

    // Top/bottom splits between groups in the same expanded Column are valid
    // rearrangements. Restricting to siblings avoids accidental profile-specific
    // targets while still changing order and testing normalization.
    for (_, owned_groups, _, mode, _) in &columns {
        if *mode != ColumnMode::Expanded {
            continue;
        }
        for source in owned_groups {
            for target in owned_groups {
                if source == target {
                    continue;
                }
                for zone in [DropZone::Top, DropZone::Bottom] {
                    commands.push(DockCommand::SplitGroup {
                        group: *source,
                        target_group: *target,
                        zone,
                        ratio: 0.5,
                    });
                }
            }
        }
    }

    for (column, owned_groups, _, _, ratios) in &columns {
        let height = 1000.0
            + owned_groups.len() as f32
                * (metrics.min_group_height + metrics.collapsed_group_height + 1.0);
        commands.push(DockCommand::ResetColumnRatios {
            column: *column,
            available_height: height,
        });
        commands.push(DockCommand::RestoreGroupRatios {
            column: *column,
            ratios: ratios.clone(),
        });
        if owned_groups.len() > 1 {
            commands.push(DockCommand::ResizeGroups {
                column: *column,
                boundary_index: 0,
                delta: 1.0,
                available_height: height,
            });
        }
    }

    // Right-dock outer column width and column-boundary resize are generated
    // only when their current structural preconditions exist.
    if let Some((column, _, width, _, _)) = right_columns.first() {
        commands.push(DockCommand::ResizeColumn {
            column: *column,
            width: *width + 1.0,
        });
    }
    if let Some(area) = layout.main_area(DockSide::Right) {
        if let Some(Node::Split(split)) = layout.node(area.root) {
            if split.axis == Axis::X && right_columns.len() > 1 {
                for boundary_index in 0..right_columns.len() - 1 {
                    commands.push(DockCommand::ResizeColumnBoundary {
                        side: DockSide::Right,
                        boundary_index,
                        delta: 1.0,
                    });
                }
            }
        }
    }

    for (column, _, _, mode, _) in &columns {
        commands.push(DockCommand::SetColumnMode {
            column: *column,
            mode: if *mode == ColumnMode::Expanded {
                ColumnMode::IconsOnly
            } else {
                ColumnMode::Expanded
            },
        });
    }
    for (column, owned_groups, _, mode, _) in &columns {
        if *mode == ColumnMode::IconsOnly {
            for group in owned_groups {
                if matches!(layout.node(*group), Some(Node::Tabs(_))) {
                    commands.push(DockCommand::OpenFlyout {
                        column: *column,
                        group: *group,
                    });
                }
            }
        }
    }

    // Floating commands use current FloatingGroupIds and current roots only.
    for (floating_id, floating) in layout.floating_groups() {
        commands.push(DockCommand::MoveFloating {
            group: floating_id,
            origin: Point::new(
                floating.bounds.origin.x + 1.0,
                floating.bounds.origin.y + 1.0,
            ),
        });
        commands.push(DockCommand::ResizeFloating {
            group: floating_id,
            size: Size::new(
                floating.bounds.size.width + 1.0,
                floating.bounds.size.height + 1.0,
            ),
        });
        commands.push(DockCommand::RaiseFloating { group: floating_id });

        if let Some(area) = layout.dock_area(floating.area) {
            let source_nodes = nodes_in_subtree(layout, area.root);
            for (target, children, _, _) in &main_groups {
                if !source_nodes.contains(target) {
                    commands.push(DockCommand::DockFloating {
                        group: floating_id,
                        target: DropTarget::Tabs {
                            group: *target,
                            index: children.len(),
                        },
                    });
                }
            }
        }
    }

    // Float a single panel only when its current tab group will retain a child.
    for (panel_node, _) in &panels {
        let Some((_, children, _, _)) = groups
            .iter()
            .find(|(_, children, _, _)| children.contains(panel_node))
        else {
            continue;
        };
        if children.len() > 1 {
            commands.push(DockCommand::Float {
                node: *panel_node,
                bounds: Rect::new(Point::new(40.0, 50.0), Size::new(320.0, 220.0)),
            });
        }
    }

    // The list is always non-empty due to CloseFlyout and SetChrome.
    commands
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use slotmap::Key;

    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 48,
            ..ProptestConfig::default()
        })]

        #[test]
        fn generated_command_sequences_from_current_ids_remain_valid(
            choices in prop::collection::vec(any::<u16>(), 1..36)
        ) {
            let metrics = DockMetrics::default();
            let mut layout = sequence_layout();
            let mut serial = 0usize;

            for choice in choices {
                let commands = valid_commands(&layout, &metrics, &mut serial);
                prop_assert!(!commands.is_empty());

                let command = commands[usize::from(choice) % commands.len()].clone();
                let result = layout.apply(command.clone(), &metrics);
                prop_assert!(
                    result.is_ok(),
                    "current-state command {command:?} was rejected: {:?}",
                    result.err()
                );
                prop_assert_eq!(
                    layout.validate(&metrics),
                    Ok(()),
                    "Layout invalid after successful command {command:?}"
                );
            }
        }
    }
}
