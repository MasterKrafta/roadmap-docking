//! C4.7 regression coverage for command failures and generated command sequences.
//!
//! Commands are assembled from the current Layout views rather than retaining
//! IDs across mutations. This exercises the transactional API as a UI adapter
//! would use it.

use std::collections::HashSet;

use slotmap::Key;

use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ChromeMode, ColumnMode, ComputeRectsError, DockCommand, DockMetrics, DockSide,
    DropTarget, DropZone, Layout, Node, NodeId, PanelConstraints, PanelId, PanelKey, Point,
    Profile, Rect, Size, UndoRedoHistory,
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
    sequence_layout_for(Profile::Generic)
}

fn photoshop_sequence_layout() -> Layout {
    sequence_layout_for(Profile::Photoshop)
}

fn sequence_layout_for(profile: Profile) -> Layout {
    let mut builder = LayoutBuilder::new(profile);
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
        columns.push(builder.column(280.0, ColumnMode::Expanded, groups, vec![0.5, 0.5]));
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

fn is_group_in_icon_rail(layout: &Layout, group: NodeId) -> bool {
    [DockSide::Left, DockSide::Right]
        .into_iter()
        .flat_map(|side| main_columns(layout, side))
        .any(|(_, groups, _, mode, _)| mode.is_rail() && groups.contains(&group))
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
        .filter(|(group, _, _, _)| {
            main_nodes.contains(group) && !is_group_in_icon_rail(layout, *group)
        })
        .collect()
}

fn assert_rejected_atomically(layout: &mut Layout, command: DockCommand, metrics: &DockMetrics) {
    let description = format!("{command:?}");
    let before = format!("{layout:?}");
    let version = layout.version();

    let result = layout.apply(command, metrics);
    assert!(
        result.is_err(),
        "expected {description} to be rejected, got {result:?}"
    );
    assert_eq!(
        format!("{layout:?}"),
        before,
        "{description} mutated Layout"
    );
    assert_eq!(layout.version(), version, "{description} changed version");
    assert_eq!(
        layout.validate(metrics),
        Ok(()),
        "rollback left invalid Layout"
    );
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
        DockCommand::MovePanelFromFlyout {
            panel: PanelId::null(),
            target,
        },
        DockCommand::FloatPanelFromFlyout {
            panel: PanelId::null(),
            bounds,
        },
        DockCommand::MoveGroup {
            group: NodeId::null(),
            column,
            index: 0,
        },
        DockCommand::MoveGroupToMainColumn {
            group: NodeId::null(),
            side: DockSide::Right,
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
            panel: PanelId::null(),
        },
    ];

    for command in commands {
        assert_rejected_atomically(&mut layout, command, &metrics);
    }

    // Flyout completion commands are exercised through drag/drop tests;
    // CloseFlyout, DismissFlyout, policy changes and ResetMainColumnRatios are
    // covered through their explicit no-op and transition tests.
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
        DockCommand::ActivateTab {
            group: column,
            panel,
        },
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
        DockCommand::MoveGroupToMainColumn {
            group: column,
            side: DockSide::Right,
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
        DockCommand::OpenFlyout { column, panel },
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

    // A complete Photoshop dock column can be floated as a composition.
    // Keep this valid operation out of the wrong-node-type rejection list.
    let mut floating_column_layout = layout.clone();
    floating_column_layout
        .apply(
            DockCommand::Float {
                node: column,
                bounds: Rect::new(Point::new(20.0, 30.0), Size::new(320.0, 220.0)),
            },
            &metrics,
        )
        .expect("a Photoshop dock column can become a floating composition");
    assert_eq!(floating_column_layout.validate(&metrics), Ok(()));

    // A flyout requires a rail column and a panel represented by one of its icons.
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
            panel: PanelId::null(),
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

fn resize_groups_preconditions_hold(
    layout: &Layout,
    groups: &[NodeId],
    ratios: &[f32],
    available_height: f32,
    metrics: &DockMetrics,
) -> bool {
    if groups.is_empty()
        || groups.len() != ratios.len()
        || !available_height.is_finite()
        || available_height <= 0.0
    {
        return false;
    }

    let mut minimums = Vec::with_capacity(groups.len());
    let mut collapsed = Vec::with_capacity(groups.len());
    let mut fixed_height = 0.0_f32;
    let mut expanded_weight_sum = 0.0_f32;
    let mut expanded_count = 0usize;

    for (group, ratio) in groups.iter().zip(ratios) {
        let Ok(minimum) = layout.minimum_node_size(*group, metrics) else {
            return false;
        };
        let is_collapsed = matches!(layout.node(*group), Some(Node::Tabs(tabs)) if tabs.collapsed);
        minimums.push(minimum.height);
        collapsed.push(is_collapsed);
        if is_collapsed {
            fixed_height += minimum.height;
        } else {
            expanded_weight_sum += *ratio;
            expanded_count += 1;
        }
    }

    let required_height: f32 = minimums.iter().sum();
    if !required_height.is_finite() || available_height < required_height {
        return false;
    }

    let free_height = available_height - fixed_height;
    if !free_height.is_finite() || free_height < 0.0 {
        return false;
    }
    if expanded_count == 0 {
        return true;
    }

    for index in 0..groups.len() {
        if collapsed[index] {
            continue;
        }
        let height = if expanded_weight_sum <= f32::EPSILON {
            free_height / expanded_count as f32
        } else {
            free_height * ratios[index] / expanded_weight_sum
        };
        // Match ResizeGroups' precondition: it rejects a current ratio layout
        // when an expanded group's proportional height is below its minimum.
        if !height.is_finite() || height + 1.0e-3 < minimums[index] {
            return false;
        }
    }

    true
}

fn valid_commands(layout: &Layout, metrics: &DockMetrics, serial: &mut usize) -> Vec<DockCommand> {
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
                ChromeMode::AllHidden
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
            if let Some(panel_id) =
                children
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
    if panels.len() > 4
        && let Some((_, panel_id)) = panels.first()
    {
        commands.push(DockCommand::ClosePanel { panel: *panel_id });
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
            if Some(*target_group) != source_group && !is_group_in_icon_rail(layout, *target_group)
            {
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
            // Generic columns may contain a Split node after a vertical
            // SplitGroup command. MoveGroup deliberately accepts Tabs only.
            if !matches!(layout.node(*source_group), Some(Node::Tabs(_))) {
                continue;
            }
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
            if !matches!(layout.node(*source), Some(Node::Tabs(_))) {
                continue;
            }
            for target in owned_groups {
                if source == target || !matches!(layout.node(*target), Some(Node::Tabs(_))) {
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
        if owned_groups.len() > 1
            && resize_groups_preconditions_hold(
                layout,
                owned_groups,
                ratios,
                height,
                metrics,
            )
        {
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
    if let Some(area) = layout.main_area(DockSide::Right)
        && let Some(Node::Split(split)) = layout.node(area.root)
        && split.axis == Axis::X
        && right_columns.len() > 1
    {
        for boundary_index in 0..right_columns.len() - 1 {
            commands.push(DockCommand::ResizeColumnBoundary {
                side: DockSide::Right,
                boundary_index,
                delta: 1.0,
            });
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
        if mode.is_rail() {
            let display_order = layout.column_display_order(*column).unwrap_or_default();
            for group in owned_groups {
                if let Some(item) = display_order.iter().find(|item| item.group == *group) {
                    commands.push(DockCommand::OpenFlyout {
                        column: *column,
                        panel: item.panel,
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
            match layout.node(area.root) {
                Some(Node::Tabs(_)) | Some(Node::Panel(_)) => {
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
                Some(Node::Split(split)) if split.axis == Axis::Y => {
                    for side in [DockSide::Left, DockSide::Right] {
                        let target_columns = main_columns(layout, side);
                        commands.push(DockCommand::DockFloating {
                            group: floating_id,
                            target: DropTarget::MainColumn {
                                side,
                                index: target_columns.len(),
                            },
                        });
                        for (column, groups, _, mode, _) in &target_columns {
                            if mode.is_rail() {
                                commands.push(DockCommand::DockFloating {
                                    group: floating_id,
                                    target: DropTarget::RailSlot {
                                        column: *column,
                                        index: groups.len(),
                                    },
                                });
                            }
                        }
                    }
                }
                // The core intentionally rejects docking an X-axis floating
                // split directly; do not generate a false-positive command.
                Some(Node::Split(_)) | None => {}
                Some(Node::Column(_)) => {}
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

    use super::*;

    #[test]
    fn generated_resize_groups_skip_columns_with_under_minimum_ratios() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(crate::Profile::Generic);

        let (_, first_panel) =
            builder.panel(PanelKey::new("generated-resize-small").unwrap(), constraints());
        let (_, second_panel) =
            builder.panel(PanelKey::new("generated-resize-large").unwrap(), constraints());
        let first_group = builder.tabs(vec![first_panel], 0, false);
        let second_group = builder.tabs(vec![second_panel], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            vec![first_group, second_group],
            vec![0.001, 0.999],
        );
        builder.main_area(DockSide::Right, column);
        let layout = builder.build();

        assert_eq!(layout.validate(&metrics), Ok(()));
        let mut serial = 0;
        let commands = valid_commands(&layout, &metrics, &mut serial);
        assert!(
            !commands.iter().any(
                |command| matches!(command, DockCommand::ResizeGroups { column: target, .. } if *target == column)
            ),
            "the generator must not label a ResizeGroups command valid when a group starts below its minimum"
        );
    }

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
                prop_assert!(
                    layout.validate(&metrics).is_ok(),
                    "Layout invalid after successful command {:?}",
                    command
                );

                // Generated commands must leave a layout that not only validates,
                // but also resolves into usable geometry in a deliberately generous
                // viewport. This keeps expected small-window failures separate from
                // model/geometry regressions.
                let viewport = Rect::new(Point::ZERO, Size::new(32_768.0, 32_768.0));
                let rects = layout.compute_rects(viewport, &metrics);
                prop_assert!(
                    rects.is_ok(),
                    "geometry failed after successful command {:?}: {:?}",
                    command,
                    rects
                );
                let mut rects = rects.expect("geometry success was asserted");

                // Drop-zone rectangles are computed separately because their
                // legality depends on the dragged source. Include a real source
                // so the snapshot checker also validates every emitted drop zone.
                let panels = panel_nodes(&layout);
                prop_assert!(!panels.is_empty(), "generated layout lost every panel");
                let drop_zones = layout.compute_drop_zones(
                    crate::DropSource::Panel { panel: panels[0].1 },
                    &rects,
                    &metrics,
                );
                prop_assert!(
                    drop_zones.is_ok(),
                    "drop-zone geometry failed after successful command {:?}: {:?}",
                    command,
                    drop_zones
                );
                rects.drop_zones = drop_zones.expect("drop-zone success was asserted");
                crate::layout_rects::assert_layout_rects_invariants(&rects);
            }
        }
    }
}

fn logical_node(layout: &Layout, node_id: NodeId) -> String {
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => match layout.panel_instance(panel.panel_id) {
            Some(instance) => format!(
                "Panel(key={:?},constraints={:?},remembered_size={:?})",
                instance.key.as_str(),
                instance.constraints,
                instance.state.last_known_size()
            ),
            None => "Panel(missing-instance)".to_owned(),
        },
        Some(Node::Tabs(tabs)) => format!(
            "Tabs(active={},collapsed={},children=[{}])",
            tabs.active_index,
            tabs.collapsed,
            tabs.children
                .iter()
                .map(|child| logical_node(layout, *child))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Some(Node::Split(split)) => format!(
            "Split(axis={:?},ratios={:?},children=[{}])",
            split.axis,
            split.ratios,
            split
                .children
                .iter()
                .map(|child| logical_node(layout, *child))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Some(Node::Column(column)) => format!(
            "Column(width={},mode={:?},ratios={:?},groups=[{}])",
            column.width,
            column.mode,
            column.group_ratios,
            column
                .groups
                .iter()
                .map(|group| logical_node(layout, *group))
                .collect::<Vec<_>>()
                .join(",")
        ),
        None => "MissingNode".to_owned(),
    }
}

/// Semantic projection of Layout: contains no NodeId, PanelId, DockAreaId or
/// FloatingGroupId values. Runtime handle identity is intentionally not part
/// of logical layout equivalence.
fn logical_snapshot(layout: &Layout) -> String {
    let mut parts = vec![format!(
        "profile={:?};chrome={:?};canvas={:?}",
        layout.profile(),
        layout.chrome(),
        layout.canvas()
    )];

    for side in [DockSide::Left, DockSide::Right] {
        let root = layout
            .main_area(side)
            .map(|area| logical_node(layout, area.root))
            .unwrap_or_else(|| "Empty".to_owned());
        parts.push(format!("main({side:?})={root}"));
    }

    let mut floating = layout
        .floating_groups()
        .filter_map(|(_, group)| {
            layout
                .dock_area(group.area)
                .map(|area| (group.z_order, group.bounds, logical_node(layout, area.root)))
        })
        .collect::<Vec<_>>();
    floating.sort_by_key(|(z_order, _, _)| *z_order);
    parts.push(format!("floating={floating:?}"));

    let flyout = layout.flyout().map_or_else(
        || "none".to_owned(),
        |flyout| {
            format!(
                "column={},group={}",
                logical_node(layout, flyout.column),
                logical_node(layout, flyout.group)
            )
        },
    );
    parts.push(format!("flyout={flyout}"));

    parts.join("\n")
}

fn persistent_logical_snapshot(layout: &Layout) -> String {
    logical_snapshot(layout)
        .lines()
        .filter(|line| !line.starts_with("flyout="))
        .collect::<Vec<_>>()
        .join("\n")
}

type RuntimeIdSets = (
    HashSet<NodeId>,
    HashSet<PanelId>,
    HashSet<crate::DockAreaId>,
    HashSet<crate::FloatingGroupId>,
);

fn runtime_id_sets(layout: &Layout) -> RuntimeIdSets {
    (
        layout.nodes().map(|(id, _)| id).collect(),
        layout.panel_instances().map(|(id, _)| id).collect(),
        layout.dock_areas().map(|(id, _)| id).collect(),
        layout.floating_groups().map(|(id, _)| id).collect(),
    )
}

fn assert_layout_state_invariants(layout: &Layout, metrics: &DockMetrics) {
    assert_eq!(layout.validate(metrics), Ok(()), "layout must validate");

    for (node_id, node) in layout.nodes() {
        match node {
            Node::Panel(panel) => assert!(
                layout.panel_instance(panel.panel_id).is_some(),
                "panel node {node_id:?} references missing PanelId {:?}",
                panel.panel_id
            ),
            Node::Split(split) => {
                for child in &split.children {
                    assert!(
                        layout.node(*child).is_some(),
                        "split {node_id:?} references missing child {child:?}"
                    );
                }
            }
            Node::Tabs(tabs) => {
                for child in &tabs.children {
                    assert!(
                        layout.node(*child).is_some(),
                        "tabs {node_id:?} references missing child {child:?}"
                    );
                }
            }
            Node::Column(column) => {
                for group in &column.groups {
                    assert!(
                        layout.node(*group).is_some(),
                        "column {node_id:?} references missing group {group:?}"
                    );
                }
            }
        }
    }

    for (area_id, area) in layout.dock_areas() {
        assert!(
            layout.node(area.root).is_some(),
            "dock area {area_id:?} references missing root {:?}",
            area.root
        );
    }
    for (floating_id, floating) in layout.floating_groups() {
        assert!(
            layout.dock_area(floating.area).is_some(),
            "floating group {floating_id:?} references missing dock area {:?}",
            floating.area
        );
    }
    for side in [DockSide::Left, DockSide::Right] {
        if let Some(area) = layout.main_area(side) {
            assert!(
                layout.node(area.root).is_some(),
                "{side:?} main area references missing root {:?}",
                area.root
            );
        }
    }
    if let Some(flyout) = layout.flyout() {
        assert!(
            layout.node(flyout.column).is_some(),
            "flyout references missing column {:?}",
            flyout.column
        );
        assert!(
            layout.node(flyout.group).is_some(),
            "flyout references missing group {:?}",
            flyout.group
        );
    }

    let rects = layout
        .compute_rects(Rect::new(Point::ZERO, Size::new(32_768.0, 32_768.0)), metrics)
        .unwrap_or_else(|error| panic!("valid layout failed large-viewport geometry: {error}"));
    crate::layout_rects::assert_layout_rects_invariants(&rects);
}

fn assert_retired_ids_stay_stale(
    layout: &Layout,
    before: &RuntimeIdSets,
    after: &RuntimeIdSets,
    context: &str,
) {
    for id in before
        .0
        .difference(&after.0)
        .chain(after.0.difference(&before.0))
    {
        assert!(
            layout.node(*id).is_none(),
            "{context}: retired/replaced NodeId {id:?} became live again"
        );
    }
    for id in before
        .1
        .difference(&after.1)
        .chain(after.1.difference(&before.1))
    {
        assert!(
            layout.panel_instance(*id).is_none(),
            "{context}: retired/replaced PanelId {id:?} became live again"
        );
    }
    for id in before
        .2
        .difference(&after.2)
        .chain(after.2.difference(&before.2))
    {
        assert!(
            layout.dock_area(*id).is_none(),
            "{context}: retired/replaced DockAreaId {id:?} became live again"
        );
    }
    for id in before
        .3
        .difference(&after.3)
        .chain(after.3.difference(&before.3))
    {
        assert!(
            layout.floating_group(*id).is_none(),
            "{context}: retired/replaced FloatingGroupId {id:?} became live again"
        );
    }
}

fn first_valid_command(
    layout: &Layout,
    metrics: &DockMetrics,
    serial: &mut usize,
    predicate: impl Fn(&DockCommand) -> bool,
) -> DockCommand {
    valid_commands(layout, metrics, serial)
        .into_iter()
        .find(predicate)
        .expect("test fixture must expose a valid command for the requested operation")
}

fn assert_history_round_trip(
    layout: &mut Layout,
    history: &mut UndoRedoHistory,
    command: DockCommand,
    metrics: &DockMetrics,
    expect_persistent_change: bool,
) {
    let description = format!("{command:?}");
    let before_snapshot = persistent_logical_snapshot(layout);
    let before_ids = runtime_id_sets(layout);
    let before_version = layout.version();
    let undo_len = history.undo_len();
    let redo_len = history.redo_len();

    let outcome = history
        .apply(layout, command.clone(), metrics)
        .unwrap_or_else(|error| panic!("history command {description} failed: {error}"));
    assert_layout_state_invariants(layout, metrics);

    if expect_persistent_change {
        assert_ne!(
            outcome.new_version, before_version,
            "{description} should make a persistent revision"
        );
        let after_snapshot = persistent_logical_snapshot(layout);
        assert_ne!(
            after_snapshot, before_snapshot,
            "{description} should change logical state"
        );
        let after_ids = runtime_id_sets(layout);

        assert!(
            history.undo(layout, metrics).unwrap(),
            "Undo must be available after {description}"
        );
        assert_eq!(
            persistent_logical_snapshot(layout),
            before_snapshot,
            "Undo failed to restore logical state for {description}"
        );
        assert_layout_state_invariants(layout, metrics);
        assert_retired_ids_stay_stale(
            layout,
            &before_ids,
            &after_ids,
            &format!("after undo of {description}"),
        );

        assert!(
            history.redo(layout, metrics).unwrap(),
            "Redo must be available after undoing {description}"
        );
        assert_eq!(
            persistent_logical_snapshot(layout),
            after_snapshot,
            "Redo failed to restore logical state for {description}"
        );
        assert_layout_state_invariants(layout, metrics);
        assert_retired_ids_stay_stale(
            layout,
            &before_ids,
            &after_ids,
            &format!("after redo of {description}"),
        );
    } else {
        assert_eq!(
            outcome.new_version, before_version,
            "{description} is transient and must not advance persistent revision"
        );
        assert_eq!(
            history.undo_len(),
            undo_len,
            "{description} must not create an undo checkpoint"
        );
        assert_eq!(
            history.redo_len(),
            redo_len,
            "{description} must not alter the redo stack"
        );
        assert_eq!(
            persistent_logical_snapshot(layout),
            before_snapshot,
            "{description} must not change persistent logical state"
        );
    }
}

#[test]
fn expected_small_viewport_errors_are_constraints_not_layout_regressions() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_node) =
        builder.panel(PanelKey::new("small-viewport-canvas").unwrap(), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    // The model is valid, but a 280px dock leaves only 319px for Canvas in a
    // 599px viewport. This is an expected sizing result, not a model failure.
    assert!(matches!(
        layout.compute_rects(
            Rect::new(Point::ZERO, Size::new(599.0, 600.0)),
            &metrics
        ),
        Err(ComputeRectsError::InsufficientCanvasWidth { available, minimum })
            if available == 319.0 && minimum == metrics.min_canvas_width
    ));

    // The same valid layout computes successfully when enough width is available.
    assert!(layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(800.0, 900.0)),
            &metrics
        )
        .is_ok());

    // A floating group's minimum content height also cannot be fitted into a
    // viewport that is shorter than the required content. Keep this failure
    // typed separately from InvalidLayout and Geometry errors.
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_node) = builder.panel(
        PanelKey::new("short-floating-viewport").unwrap(),
        constraints(),
    );
    let group = builder.tabs(vec![panel_node], 0, false);
    builder.floating_group(
        group,
        Rect::new(Point::new(30.0, 20.0), Size::new(320.0, 220.0)),
        1,
    );
    let floating_layout = builder.build();
    assert_eq!(floating_layout.validate(&metrics), Ok(()));
    let constrained_rects = floating_layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1000.0, 100.0)),
            &metrics,
        )
        .expect("an undersized floating window is isolated from the rest of geometry");
    assert_eq!(
        constrained_rects.canvas,
        Some(Rect::new(Point::ZERO, Size::new(1000.0, 100.0)))
    );
    assert!(constrained_rects.floating_groups.is_empty());
    assert!(constrained_rects.panels.is_empty());
    assert_eq!(floating_layout.validate(&metrics), Ok(()));
    assert!(floating_layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1000.0, 600.0)),
            &metrics
        )
        .is_ok());
}

#[test]
fn mutation_sequences_round_trip_through_undo_redo_without_reviving_stale_ids() {
    let metrics = DockMetrics::default();
    let mut layout = sequence_layout();
    let mut history = UndoRedoHistory::new();
    let mut serial = 0usize;

    // Move a group to another column.
    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::MoveGroup { group, column, .. } if {
            let source_column = [DockSide::Left, DockSide::Right]
                .into_iter()
                .flat_map(|side| main_columns(&layout, side))
                .find(|(_, groups, _, _, _)| groups.contains(group))
                .map(|(id, _, _, _, _)| id);
            source_column.is_some_and(|source| source != *column)
        })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::ResizeGroups { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::ResizeColumn { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::CollapseGroup { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::MovePanel { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::Float { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::MoveFloating { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::ResizeFloating { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::DockFloating { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    // Rail mode and flyout are separate transitions; flyout visibility is
    // transient and must not enter persistent undo history.
    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::SetColumnMode { mode: ColumnMode::IconsOnly, .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, true);

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::OpenFlyout { .. })
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, false);
    assert!(layout.flyout().is_some(), "OpenFlyout should set transient state");

    let command = first_valid_command(&layout, &metrics, &mut serial, |command| {
        matches!(command, DockCommand::CloseFlyout)
    });
    assert_history_round_trip(&mut layout, &mut history, command, &metrics, false);
    assert!(layout.flyout().is_none(), "CloseFlyout should clear transient state");
}

#[cfg(feature = "serde")]
fn floating_group_for_panel(layout: &Layout, wanted_panel: PanelId) -> crate::FloatingGroupId {
    layout
        .floating_groups()
        .find_map(|(floating_id, floating)| {
            let area = layout.dock_area(floating.area)?;
            let nodes = nodes_in_subtree(layout, area.root);
            nodes
                .iter()
                .any(|node_id| {
                    matches!(layout.node(*node_id), Some(Node::Panel(panel)) if panel.panel_id == wanted_panel)
                })
                .then_some(floating_id)
        })
        .expect("floating group contains the requested panel")
}
fn panel_id_for_key(layout: &Layout, wanted: &str) -> PanelId {
    layout
        .panel_instances()
        .find_map(|(panel_id, instance)| (instance.key.as_str() == wanted).then_some(panel_id))
        .expect("test layout contains the requested panel key")
}

fn panel_node_for_key(layout: &Layout, wanted: &str) -> NodeId {
    let panel_id = panel_id_for_key(layout, wanted);
    layout
        .nodes()
        .find_map(|(node_id, node)| match node {
            Node::Panel(panel) if panel.panel_id == panel_id => Some(node_id),
            _ => None,
        })
        .expect("test layout contains a leaf for the requested panel key")
}

fn reordered_id_layout() -> Layout {
    // Same logical tree and panel descriptors as create_initial_layout(), but
    // allocate runtime instances/leaves in reverse key order.
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, history) = builder.panel(PanelKey::new("history").unwrap(), constraints());
    let (_, properties) = builder.panel(PanelKey::new("properties").unwrap(), constraints());
    let (_, color) = builder.panel(PanelKey::new("color").unwrap(), constraints());
    let (_, layers) = builder.panel(PanelKey::new("layers").unwrap(), constraints());

    let appearance = builder.tabs(vec![layers, color], 0, false);
    let information = builder.tabs(vec![properties, history], 0, false);
    let root = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![appearance, information],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, root);
    builder.build()
}

#[cfg(feature = "serde")]
fn apply_and_record(
    layout: &mut Layout,
    journal: &mut Vec<DockCommand>,
    command: DockCommand,
    metrics: &DockMetrics,
) {
    let result = layout.apply(command.clone(), metrics);
    assert!(
        result.is_ok(),
        "journal command {command:?} was rejected: {:?}",
        result.err()
    );
    assert_eq!(
        layout.validate(metrics),
        Ok(()),
        "Layout invalid after journal command {command:?}"
    );
    journal.push(command);
}

#[test]
fn c47_logical_equivalence_ignores_runtime_id_allocation_order() {
    let factory_layout = initial_layout(Profile::Generic);
    let reordered_layout = reordered_id_layout();

    assert_eq!(factory_layout.validate(&DockMetrics::default()), Ok(()));
    assert_eq!(reordered_layout.validate(&DockMetrics::default()), Ok(()));
    assert_ne!(
        panel_id_for_key(&factory_layout, "layers"),
        panel_id_for_key(&reordered_layout, "layers"),
        "test setup must assign different runtime PanelIds to the same logical panel"
    );
    assert_ne!(
        panel_node_for_key(&factory_layout, "layers"),
        panel_node_for_key(&reordered_layout, "layers"),
        "test setup must assign a different runtime leaf NodeId to the same panel key"
    );
    assert_eq!(
        logical_snapshot(&factory_layout),
        logical_snapshot(&reordered_layout)
    );
}

#[cfg(feature = "serde")]
#[test]
fn c47_serialized_command_journal_replays_to_equivalent_logical_layout() {
    let metrics = DockMetrics::default();
    let mut recorded = sequence_layout();
    let mut journal = Vec::new();

    let initial_columns = main_columns(&recorded, DockSide::Right);
    assert_eq!(initial_columns.len(), 2);
    let first_column = initial_columns[0].0;
    let first_group = initial_columns[0].1[0];
    let second_group = initial_columns[0].1[1];
    let other_column_group = initial_columns[1].1[0];
    let initial_children = tab_groups(&recorded)
        .into_iter()
        .find(|(group, _, _, _)| *group == first_group)
        .unwrap()
        .1;
    assert_eq!(initial_children.len(), 2);

    let second_panel = match recorded.node(initial_children[1]).unwrap() {
        Node::Panel(panel) => panel.panel_id,
        _ => panic!("test tabs initially contain panel leaves"),
    };
    let first_panel = match recorded.node(initial_children[0]).unwrap() {
        Node::Panel(panel) => panel.panel_id,
        _ => panic!("test tabs initially contain panel leaves"),
    };

    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::ActivateTab {
            group: first_group,
            panel: second_panel,
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::ReorderTab {
            group: first_group,
            panel: first_panel,
            to_index: 1,
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::CollapseGroup {
            group: other_column_group,
            collapsed: true,
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::ResizeGroups {
            column: first_column,
            boundary_index: 0,
            delta: 20.0,
            available_height: 1200.0,
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::MovePanel {
            panel: first_panel,
            target: DropTarget::Tabs {
                group: second_group,
                index: 2,
            },
        },
        &metrics,
    );

    // Float the remaining panel, then dock it back. This also exercises
    // generation changes when a removed FloatingGroup slot is reused.
    let remaining_panel_node = tab_groups(&recorded)
        .into_iter()
        .find(|(group, _, _, _)| *group == first_group)
        .and_then(|(_, children, _, _)| children.first().copied())
        .expect("source tabs retains its second panel");
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::Float {
            node: remaining_panel_node,
            bounds: Rect::new(Point::new(45.0, 65.0), Size::new(320.0, 220.0)),
        },
        &metrics,
    );

    let temporary_floating_id = recorded.floating_groups().next().unwrap().0;
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::DockFloating {
            group: temporary_floating_id,
            target: DropTarget::Tabs {
                group: second_group,
                index: 3,
            },
        },
        &metrics,
    );

    // Leave two windows floating in the final state so the semantic snapshot
    // compares their bounds and z-order without comparing FloatingGroupIds.
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::Float {
            node: remaining_panel_node,
            bounds: Rect::new(Point::new(220.0, 120.0), Size::new(320.0, 220.0)),
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::Float {
            node: initial_children[0],
            bounds: Rect::new(Point::new(420.0, 180.0), Size::new(300.0, 240.0)),
        },
        &metrics,
    );

    let first_floating = floating_group_for_panel(&recorded, second_panel);
    let second_floating = floating_group_for_panel(&recorded, first_panel);
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::MoveFloating {
            group: first_floating,
            origin: Point::new(238.0, 132.0),
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::ResizeFloating {
            group: first_floating,
            size: Size::new(360.0, 260.0),
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::MoveFloating {
            group: second_floating,
            origin: Point::new(450.0, 195.0),
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::ResizeFloating {
            group: second_floating,
            size: Size::new(340.0, 250.0),
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::RaiseFloating {
            group: first_floating,
        },
        &metrics,
    );
    apply_and_record(
        &mut recorded,
        &mut journal,
        DockCommand::SetChrome {
            mode: ChromeMode::AllHidden,
        },
        &metrics,
    );

    let encoded_journal =
        serde_json::to_string(&journal).expect("DockCommand journal is serializable with serde");
    let decoded_journal: Vec<DockCommand> = serde_json::from_str(&encoded_journal)
        .expect("serialized DockCommand journal decodes without losing intent");

    let mut replayed = sequence_layout();
    for command in decoded_journal {
        let result = replayed.apply(command.clone(), &metrics);
        assert!(
            result.is_ok(),
            "replayed journal command {command:?} was rejected: {:?}",
            result.err()
        );
        assert_eq!(
            replayed.validate(&metrics),
            Ok(()),
            "replayed Layout invalid after {command:?}"
        );
    }

    assert_eq!(recorded.version(), replayed.version());
    assert_eq!(
        logical_snapshot(&recorded),
        logical_snapshot(&replayed),
        "replay should reproduce the same logical tree and floating geometry without comparing runtime IDs"
    );
}

#[test]
fn c47_failed_commands_preserve_full_state_and_version() {
    let metrics = DockMetrics::default();
    let mut layout = sequence_layout();
    let columns = main_columns(&layout, DockSide::Right);
    let groups = tab_groups(&layout);
    let column = columns[0].0;
    let group = columns[0].1[0];
    let children = groups
        .iter()
        .find(|(id, _, _, _)| *id == group)
        .unwrap()
        .1
        .clone();
    let panel_id = match layout.node(children[0]).unwrap() {
        Node::Panel(panel) => panel.panel_id,
        _ => panic!("test tabs contain panel leaves"),
    };

    // Set up a valid floating window so its invalid resize/move/dock commands
    // exercise transaction rollback against a state with multiple area kinds.
    layout
        .apply(
            DockCommand::Float {
                node: children[0],
                bounds: Rect::new(Point::new(40.0, 50.0), Size::new(320.0, 220.0)),
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.validate(&metrics), Ok(()));
    let floating_id = layout.floating_groups().next().unwrap().0;

    let invalid_constraints = PanelConstraints::new(
        Size::new(300.0, 80.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        false,
    );
    let invalid_commands = vec![
        DockCommand::OpenPanel {
            key: PanelKey::new("c47-invalid-constraints").unwrap(),
            constraints: invalid_constraints,
            target: Some(DropTarget::Tabs { group, index: 0 }),
        },
        DockCommand::OpenPanel {
            key: PanelKey::new("c47-invalid-index").unwrap(),
            constraints: constraints(),
            target: Some(DropTarget::Tabs {
                group,
                index: usize::MAX,
            }),
        },
        DockCommand::SplitGroup {
            group,
            target_group: columns[0].1[1],
            zone: DropZone::Bottom,
            ratio: 0.0,
        },
        DockCommand::ResizeGroups {
            column,
            boundary_index: 0,
            delta: 1.0,
            available_height: 1.0,
        },
        DockCommand::ResizeColumn {
            column,
            width: f32::NAN,
        },
        DockCommand::ResizeColumnBoundary {
            side: DockSide::Right,
            boundary_index: usize::MAX,
            delta: 1.0,
        },
        DockCommand::ResetColumnRatios {
            column,
            available_height: 1.0,
        },
        DockCommand::RestoreGroupRatios {
            column,
            ratios: vec![1.0],
        },
        DockCommand::MoveFloating {
            group: floating_id,
            origin: Point::new(f32::NAN, 1.0),
        },
        DockCommand::ResizeFloating {
            group: floating_id,
            size: Size::ZERO,
        },
        DockCommand::DockFloating {
            group: floating_id,
            target: DropTarget::Tabs {
                group: NodeId::null(),
                index: 0,
            },
        },
        DockCommand::Float {
            node: column,
            bounds: Rect::new(Point::ZERO, Size::ZERO),
        },
        DockCommand::MoveGroup {
            group: children[0],
            column,
            index: 0,
        },
        DockCommand::ClosePanel {
            panel: PanelId::null(),
        },
    ];

    for command in invalid_commands {
        let before = format!("{layout:?}");
        let version_before = layout.version();
        let result = layout.apply(command.clone(), &metrics);
        assert!(result.is_err(), "expected command to fail: {command:?}");
        assert_eq!(
            format!("{layout:?}"),
            before,
            "failed command changed some part of Layout: {command:?}"
        );
        assert_eq!(
            layout.version(),
            version_before,
            "failed command advanced version: {command:?}"
        );
        assert_eq!(
            layout.validate(&metrics),
            Ok(()),
            "rollback left Layout invalid after {command:?}"
        );
    }

    assert!(layout.panel_instance(panel_id).is_some());
}


#[test]
fn float_group_as_column_preserves_group_and_wraps_it_in_one_column() {
    let metrics = DockMetrics::default();
    let mut layout = photoshop_sequence_layout();
    let (source_column, source_groups, _, _, _) =
        main_columns(&layout, DockSide::Right)[0].clone();
    let moved_group = source_groups[0];
    let bounds = Rect::new(Point::new(25.0, 35.0), Size::new(280.0, 360.0));

    layout
        .apply(
            DockCommand::FloatGroupAsColumn {
                group: moved_group,
                bounds,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.validate(&metrics), Ok(()));
    let (_, floating) = layout.floating_groups().next().expect("floating group");
    let root = layout.dock_area(floating.area).expect("floating area").root;
    let Some(Node::Column(floating_column)) = layout.node(root) else {
        panic!("detached tab group must have a Column root");
    };
    assert_eq!(floating_column.groups, vec![moved_group]);
    assert_eq!(floating_column.group_ratios, vec![1.0]);
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(column)) if !column.groups.contains(&moved_group)
    ));
}

#[test]
fn float_panel_as_group_creates_a_one_tab_group_inside_a_column() {
    let metrics = DockMetrics::default();
    let mut layout = photoshop_sequence_layout();
    let (_, source_groups, _, _, _) =
        main_columns(&layout, DockSide::Right)[0].clone();
    let source_group = source_groups[0];
    let source_panel_node = match layout.node(source_group) {
        Some(Node::Tabs(tabs)) if tabs.children.len() > 1 => tabs.children[0],
        _ => panic!("test source group must contain multiple tabs"),
    };
    let panel_id = match layout.node(source_panel_node) {
        Some(Node::Panel(panel)) => panel.panel_id,
        _ => panic!("test source tab must be a Panel"),
    };
    let bounds = Rect::new(Point::new(45.0, 55.0), Size::new(280.0, 360.0));

    layout
        .apply(
            DockCommand::FloatPanelAsGroup {
                panel: panel_id,
                bounds,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.validate(&metrics), Ok(()));
    let (_, floating) = layout.floating_groups().next().expect("floating group");
    let root = layout.dock_area(floating.area).expect("floating area").root;
    let Some(Node::Column(floating_column)) = layout.node(root) else {
        panic!("detached tab must have a Column root");
    };
    assert_eq!(floating_column.groups.len(), 1);
    let one_tab_group = floating_column.groups[0];
    let Some(Node::Tabs(tabs)) = layout.node(one_tab_group) else {
        panic!("detached tab must be inside a Tabs node");
    };
    assert_eq!(tabs.children, vec![source_panel_node]);
    assert_eq!(tabs.active_index, 0);
    assert!(!tabs.collapsed);
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs)) if tabs.children.len() == 1 && !tabs.children.contains(&source_panel_node)
    ));
}

#[test]
fn float_group_as_column_rejects_detaching_the_last_group_atomically() {
    let metrics = DockMetrics::default();
    let mut layout = photoshop_sequence_layout();
    let (column, groups, _, _, _) =
        main_columns(&layout, DockSide::Right)[0].clone();
    let retained_group = groups[1];
    layout
        .apply(
            DockCommand::CloseGroup { group: groups[0] },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(column)) if column.groups == vec![retained_group]
    ));

    assert_rejected_atomically(
        &mut layout,
        DockCommand::FloatGroupAsColumn {
            group: retained_group,
            bounds: Rect::new(Point::ZERO, Size::new(280.0, 360.0)),
        },
        &metrics,
    );
}

#[test]
fn float_panel_as_group_rejects_detaching_the_only_tab_atomically() {
    let metrics = DockMetrics::default();
    let mut layout = photoshop_sequence_layout();
    let (_, groups, _, _, _) =
        main_columns(&layout, DockSide::Right)[0].clone();
    let group = groups[0];
    let panel_nodes = match layout.node(group) {
        Some(Node::Tabs(tabs)) => tabs.children.clone(),
        _ => panic!("test group must be Tabs"),
    };
    let panel_id = match layout.node(panel_nodes[1]) {
        Some(Node::Panel(panel)) => panel.panel_id,
        _ => panic!("test tab must be a Panel"),
    };
    layout
        .apply(
            DockCommand::ClosePanel {
                panel: match layout.node(panel_nodes[0]) {
                    Some(Node::Panel(panel)) => panel.panel_id,
                    _ => panic!("test tab must be a Panel"),
                },
            },
            &metrics,
        )
        .unwrap();

    assert_rejected_atomically(
        &mut layout,
        DockCommand::FloatPanelAsGroup {
            panel: panel_id,
            bounds: Rect::new(Point::ZERO, Size::new(280.0, 360.0)),
        },
        &metrics,
    );
}
