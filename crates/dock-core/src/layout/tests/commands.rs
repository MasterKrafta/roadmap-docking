use super::*;

#[test]
fn open_panel_expands_collapsed_photoshop_target_group() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, existing_panel) = builder.panel(
        PanelKey::new("open-collapsed-existing").unwrap(),
        descriptors()[0].1,
    );
    let group = builder.tabs(vec![existing_panel], 0, true);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::OpenPanel {
                key: PanelKey::new("open-collapsed-new").unwrap(),
                constraints: descriptors()[0].1,
                target: Some(DropTarget::Tabs { group, index: 1 }),
            },
            &metrics,
        )
        .unwrap();

    assert!(
        matches!(layout.node(group), Some(Node::Tabs(tabs)) if !tabs.collapsed && tabs.children.len() == 2)
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn apply_commits_a_successful_change_and_increments_version_once() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Generic);
    let column_id = layout.main_area(DockSide::Right).unwrap().root;
    let old_width = match layout.node(column_id).unwrap() {
        Node::Column(column) => column.width,
        _ => panic!("initial main root must be a Column"),
    };

    let outcome = layout
        .apply(
            DockCommand::ResizeColumn {
                column: column_id,
                width: old_width + 24.0,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.version(), 1);
    assert_eq!(outcome.new_version, 1);
    assert!(outcome.changed_nodes.contains(&column_id));
    assert_eq!(
        outcome.actual_value,
        Some(CommandValue::ColumnWidth(old_width + 24.0))
    );
    assert!(outcome.inverse.is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));

    // Repeating the same effective value is a no-op and must not add a version.
    let no_op = layout
        .apply(
            DockCommand::ResizeColumn {
                column: column_id,
                width: old_width + 24.0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.version(), 1);
    assert_eq!(no_op.new_version, 1);
    assert!(no_op.changed_nodes.is_empty());
    assert!(no_op.inverse.is_none());
}

#[test]
fn apply_rejection_leaves_layout_and_version_unchanged() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Generic);
    let before = format!("{layout:?}");
    let before_version = layout.version();

    let error = layout
        .apply(
            DockCommand::ResizeColumn {
                column: NodeId::null(),
                width: 300.0,
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(error, CommandError::MissingNode { .. }));
    assert_eq!(layout.version(), before_version);
    assert_eq!(format!("{layout:?}"), before);
}

#[test]
fn apply_post_operation_validation_failure_rolls_back_the_candidate() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut columns = Vec::new();

    for index in 0..metrics.max_columns {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("column-panel-{index}")).unwrap(),
            descriptors()[0].1,
        );
        let group = builder.tabs(vec![panel], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        columns.push(column);
    }

    let ratios = vec![1.0 / metrics.max_columns as f32; metrics.max_columns];
    let root = builder.split(Axis::X, columns, ratios);
    builder.main_area(DockSide::Right, root);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    let before = format!("{layout:?}");
    let error = layout
        .apply(
            DockCommand::OpenPanel {
                key: PanelKey::new("extra-panel").unwrap(),
                constraints: descriptors()[0].1,
                target: Some(DropTarget::MainColumn {
                    side: DockSide::Right,
                    index: metrics.max_columns,
                }),
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CommandError::InvalidLayout { errors }
            if errors.iter().any(|error| matches!(error, ValidationError::TooManyColumns { .. }))
    ));
    assert_eq!(layout.version(), 0);
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn apply_vertical_group_split_uses_the_requested_target_groups_ratio() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut target_groups = Vec::new();

    for index in 0..3 {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("target-panel-{index}")).unwrap(),
            descriptors()[0].1,
        );
        target_groups.push(builder.tabs(vec![panel], 0, false));
    }
    let target_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        target_groups.clone(),
        vec![0.2, 0.5, 0.3],
    );
    builder.main_area(DockSide::Right, target_column);

    let (_, source_panel) =
        builder.panel(PanelKey::new("source-panel").unwrap(), descriptors()[0].1);
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group: target_groups[1],
                zone: DropZone::Bottom,
                ratio: 0.4,
            },
            &metrics,
        )
        .unwrap();

    let Node::Column(column) = layout.node(target_column).unwrap() else {
        panic!("target column must remain a Column");
    };
    assert_eq!(
        column.groups,
        vec![
            target_groups[0],
            target_groups[1],
            source_group,
            target_groups[2]
        ]
    );
    assert_eq!(column.group_ratios, vec![0.2, 0.3, 0.2, 0.3]);
    assert!(layout.main_area(DockSide::Left).is_none());
    assert_eq!(layout.version(), 1);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn main_column_ratios_reset_visible_widths() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, first_panel) = builder.panel(
        PanelKey::new("reset-main-width-first").unwrap(),
        descriptors()[0].1,
    );
    let first_group = builder.tabs(vec![first_panel], 0, false);
    let first_column = builder.column(
        360.0,
        crate::ColumnMode::Expanded,
        vec![first_group],
        vec![1.0],
    );

    let (_, second_panel) = builder.panel(
        PanelKey::new("reset-main-width-second").unwrap(),
        descriptors()[0].1,
    );
    let second_group = builder.tabs(vec![second_panel], 0, false);
    let second_column = builder.column(
        200.0,
        crate::ColumnMode::Expanded,
        vec![second_group],
        vec![1.0],
    );

    let root = builder.split(Axis::X, vec![first_column, second_column], vec![0.9, 0.1]);
    builder.main_area(DockSide::Left, root);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    let reset = layout
        .apply(
            DockCommand::ResetMainColumnRatios {
                side: DockSide::Left,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        reset.actual_value,
        Some(CommandValue::ColumnWidths(vec![
            (first_column, 280.0),
            (second_column, 280.0),
        ]))
    );

    let (first_width, second_width) = match (layout.node(first_column), layout.node(second_column))
    {
        (Some(Node::Column(first)), Some(Node::Column(second))) => (first.width, second.width),
        _ => panic!("both main columns must remain in the layout"),
    };
    assert!((first_width - 280.0).abs() < 1.0e-3);
    assert!((second_width - 280.0).abs() < 1.0e-3);
    assert!(matches!(
        layout.node(root),
        Some(Node::Split(split)) if split.ratios == vec![0.5, 0.5]
    ));

    let rects = layout
        .compute_rects(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)), &metrics)
        .unwrap();
    for column in [first_column, second_column] {
        let rect = rects
            .columns
            .iter()
            .find(|rect| rect.column == column)
            .unwrap();
        assert!((rect.rect.size.width - 280.0).abs() < 1.0e-3);
    }
}

#[test]
fn main_column_split_group_ratio_controls_new_column_width() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(
        PanelKey::new("ratio-main-width-source").unwrap(),
        descriptors()[0].1,
    );
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(
        PanelKey::new("ratio-main-width-target").unwrap(),
        descriptors()[0].1,
    );
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        360.0,
        crate::ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group,
                zone: DropZone::Right,
                ratio: 0.4,
            },
            &metrics,
        )
        .unwrap();

    let root = layout.main_area(DockSide::Right).unwrap().root;
    let new_column = match layout.node(root) {
        Some(Node::Split(split)) => {
            assert_eq!(split.children.len(), 2);
            assert_eq!(split.ratios.len(), 2);
            assert!((split.ratios[0] - 0.6).abs() < 1.0e-4);
            assert!((split.ratios[1] - 0.4).abs() < 1.0e-4);
            split
                .children
                .iter()
                .copied()
                .find(|id| *id != target_column)
                .unwrap()
        }
        other => panic!("expected a horizontal split of main columns, found {other:?}"),
    };
    assert!(matches!(
        layout.node(target_column),
        Some(Node::Column(column)) if (column.width - 360.0).abs() < 1.0e-3
    ));
    assert!(matches!(
        layout.node(new_column),
        Some(Node::Column(column)) if (column.width - 240.0).abs() < 1.0e-3
    ));

    let rects = layout
        .compute_rects(Rect::new(Point::ZERO, Size::new(1600.0, 900.0)), &metrics)
        .unwrap();
    assert!(matches!(
        rects.columns.iter().find(|rect| rect.column == new_column),
        Some(rect) if (rect.rect.size.width - 240.0).abs() < 1.0e-3
    ));
}

#[test]
fn apply_version_overflow_rejects_commit_atomically() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Generic);
    let column_id = layout.main_area(DockSide::Right).unwrap().root;
    layout.version = u64::MAX;
    let before = format!("{layout:?}");

    let error = layout
        .apply(
            DockCommand::ResizeColumn {
                column: column_id,
                width: 320.0,
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CommandError::InvalidValue {
            field: "version",
            ..
        }
    ));
    assert_eq!(layout.version(), u64::MAX);
    assert_eq!(format!("{layout:?}"), before);
}

#[test]
fn apply_invalid_value_is_rejected_without_panicking_or_mutating_state() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Generic);
    let column_id = layout.main_area(DockSide::Right).unwrap().root;
    let before = format!("{layout:?}");

    let error = layout
        .apply(
            DockCommand::ResizeColumn {
                column: column_id,
                width: f32::NAN,
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CommandError::InvalidValue { field: "width", .. }
    ));
    assert_eq!(layout.version(), 0);
    assert_eq!(format!("{layout:?}"), before);
}

#[test]
fn apply_move_group_reorders_preserving_ratios_and_inverse_restores_order() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Generic);
    let mut groups = Vec::new();

    for index in 0..3 {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("move-group-panel-{index}")).unwrap(),
            descriptors()[0].1,
        );
        groups.push(builder.tabs(vec![panel], 0, false));
    }
    let column_id = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        groups.clone(),
        vec![0.2, 0.5, 0.3],
    );
    builder.main_area(DockSide::Right, column_id);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    let moved = layout
        .apply(
            DockCommand::MoveGroup {
                group: groups[0],
                column: column_id,
                index: 3,
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(column_id),
        Some(Node::Column(column))
            if column.groups == vec![groups[1], groups[2], groups[0]]
                && column.group_ratios == vec![0.5, 0.3, 0.2]
    ));
    assert_eq!(layout.version(), 1);

    layout
        .apply(
            moved.inverse.expect("forward move has a safe inverse"),
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(column_id),
        Some(Node::Column(column))
            if column.groups == groups && column.group_ratios == vec![0.2, 0.5, 0.3]
    ));
    assert_eq!(layout.version(), 2);

    let moved_backwards = layout
        .apply(
            DockCommand::MoveGroup {
                group: groups[2],
                column: column_id,
                index: 0,
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(column_id),
        Some(Node::Column(column))
            if column.groups == vec![groups[2], groups[0], groups[1]]
                && column.group_ratios == vec![0.3, 0.2, 0.5]
    ));
    layout
        .apply(
            moved_backwards
                .inverse
                .expect("backward move has a safe inverse"),
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(column_id),
        Some(Node::Column(column))
            if column.groups == groups && column.group_ratios == vec![0.2, 0.5, 0.3]
    ));
    assert_eq!(layout.version(), 4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn apply_drop_of_panel_at_its_current_tab_slot_is_a_no_op() {
    let metrics = DockMetrics::default();
    let mut layout = initial_layout(Profile::Generic);
    let root = layout.main_area(DockSide::Right).unwrap().root;
    let group = match layout.node(root).unwrap() {
        Node::Column(column) => column.groups[0],
        _ => panic!("initial main root must be a Column"),
    };
    let panel_id = match layout.node(group).unwrap() {
        Node::Tabs(tabs) => match layout.node(tabs.children[0]).unwrap() {
            Node::Panel(panel) => panel.panel_id,
            _ => panic!("tab entry must be a Panel"),
        },
        _ => panic!("column group must be Tabs"),
    };
    let before = format!("{layout:?}");

    let outcome = layout
        .apply(
            DockCommand::MovePanel {
                panel: panel_id,
                target: DropTarget::Tabs { group, index: 0 },
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(outcome.new_version, 0);
    assert!(outcome.changed_nodes.is_empty());
    assert!(outcome.inverse.is_none());
    assert_eq!(layout.version(), 0);
    assert_eq!(format!("{layout:?}"), before);

    let self_drop = layout
        .apply(
            DockCommand::SplitGroup {
                group,
                target_group: group,
                zone: DropZone::Right,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(self_drop.new_version, 0);
    assert!(self_drop.changed_nodes.is_empty());
    assert!(self_drop.inverse.is_none());
    assert_eq!(format!("{layout:?}"), before);
}

#[test]
fn apply_transient_flyout_does_not_increment_persistent_version() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (panel_id, panel_node) =
        builder.panel(PanelKey::new("layers").unwrap(), descriptors()[0].1);
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(
        metrics.rail_width,
        crate::ColumnMode::IconsOnly,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    let outcome = layout
        .apply(
            DockCommand::OpenFlyout {
                column,
                panel: panel_id,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(
        layout.flyout(),
        Some(crate::FlyoutState::new(column, group, panel_id))
    );
    assert_eq!(layout.version(), 0);
    assert_eq!(outcome.new_version, 0);
    assert!(outcome.inverse.is_none());
    assert!(outcome.changed_nodes.is_empty());
}

#[test]
fn c43_activate_and_reorder_tabs_preserve_active_panel_identity() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Generic);
    let (first_id, first_node) =
        builder.panel(PanelKey::new("c43-first").unwrap(), descriptors()[0].1);
    let (second_id, second_node) =
        builder.panel(PanelKey::new("c43-second").unwrap(), descriptors()[0].1);
    let (third_id, third_node) =
        builder.panel(PanelKey::new("c43-third").unwrap(), descriptors()[0].1);
    let group = builder.tabs(vec![first_node, second_node, third_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::ActivateTab {
                group,
                panel: second_id,
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(layout.node(group), Some(Node::Tabs(tabs)) if tabs.active_index == 1));
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::ReorderTab {
                group,
                panel: second_id,
                to_index: 2,
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![first_node, third_node, second_node] && tabs.active_index == 2
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::ReorderTab {
                group,
                panel: first_id,
                to_index: 1,
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![third_node, first_node, second_node] && tabs.active_index == 2
    ));
    assert_eq!(layout.panel_instances().count(), 3);
    assert!(layout.panel_instance(third_id).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c43_move_panel_between_dock_and_floating_keeps_instance_and_activates_destination() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Generic);
    let (moving_id, moving_node) =
        builder.panel(PanelKey::new("c43-moving").unwrap(), descriptors()[0].1);
    let (_, dock_sibling) = builder.panel(
        PanelKey::new("c43-dock-sibling").unwrap(),
        descriptors()[0].1,
    );
    let dock_group = builder.tabs(vec![moving_node, dock_sibling], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![dock_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);

    let (_, floating_sibling) = builder.panel(
        PanelKey::new("c43-floating-sibling").unwrap(),
        descriptors()[0].1,
    );
    let floating_root = builder.tabs(vec![floating_sibling], 0, false);
    let floating_id = builder.floating_group(
        floating_root,
        Rect::new(Point::new(80.0, 70.0), Size::new(360.0, 260.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_id,
                target: DropTarget::Tabs {
                    group: floating_root,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(floating_root),
        Some(Node::Tabs(tabs))
            if tabs.children.len() == 2 && tabs.active_index == 1
                && matches!(layout.node(tabs.children[1]), Some(Node::Panel(panel)) if panel.panel_id == moving_id)
    ));
    assert!(layout.panel_instance(moving_id).is_some());
    assert_eq!(layout.panel_instances().count(), 3);
    assert!(layout.floating_group(floating_id).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_id,
                target: DropTarget::Tabs {
                    group: dock_group,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        layout.node(dock_group),
        Some(Node::Tabs(tabs))
            if tabs.children.len() == 2 && tabs.active_index == 1
                && matches!(layout.node(tabs.children[1]), Some(Node::Panel(panel)) if panel.panel_id == moving_id)
    ));
    assert!(layout.floating_group(floating_id).is_some());
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c43_close_panel_removes_instance_and_empty_group_column_and_remembers_location() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let key = PanelKey::new("c43-last-panel").unwrap();
    let (panel_id, panel_node) = builder.panel(key.clone(), descriptors()[0].1);
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(DockCommand::ClosePanel { panel: panel_id }, &metrics)
        .unwrap();

    assert!(layout.panel_instance(panel_id).is_none());
    assert!(layout.main_area(DockSide::Right).is_none());
    assert_eq!(layout.nodes().count(), 0);
    assert_eq!(layout.panel_instances().count(), 0);
    assert_eq!(
        layout.last_dock_location(&key),
        Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c43_close_group_removes_all_owned_panel_instances_without_losing_other_groups() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let key_a = PanelKey::new("c43-group-a").unwrap();
    let key_b = PanelKey::new("c43-group-b").unwrap();
    let key_c = PanelKey::new("c43-group-c").unwrap();
    let (panel_a, node_a) = builder.panel(key_a.clone(), descriptors()[0].1);
    let (panel_b, node_b) = builder.panel(key_b.clone(), descriptors()[0].1);
    let (panel_c, node_c) = builder.panel(key_c, descriptors()[0].1);
    let closing_group = builder.tabs(vec![node_a, node_b], 0, false);
    let kept_group = builder.tabs(vec![node_c], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![closing_group, kept_group],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::CloseGroup {
                group: closing_group,
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.panel_instance(panel_a).is_none());
    assert!(layout.panel_instance(panel_b).is_none());
    assert_eq!(layout.panel_instances().count(), 1);
    assert!(layout.panel_instance(panel_c).is_some());
    assert_eq!(
        layout.last_dock_location(&key_a),
        Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
    );
    assert_eq!(
        layout.last_dock_location(&key_b),
        Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
    );
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(value)) if value.groups == vec![kept_group]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c43_open_panel_restores_runtime_location_then_falls_back_to_profile_default() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let key = PanelKey::new("c43-restore-location").unwrap();
    let (closed_id, closed_node) = builder.panel(key.clone(), descriptors()[0].1);
    let (_, sibling) = builder.panel(
        PanelKey::new("c43-restore-sibling").unwrap(),
        descriptors()[0].1,
    );
    let group = builder.tabs(vec![closed_node, sibling], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    layout
        .apply(DockCommand::ClosePanel { panel: closed_id }, &metrics)
        .unwrap();
    assert!(layout.panel_instance(closed_id).is_none());
    assert_eq!(
        layout.last_dock_location(&key),
        Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
    );

    layout
        .apply(
            DockCommand::OpenPanel {
                key: key.clone(),
                constraints: descriptors()[0].1,
                target: None,
            },
            &metrics,
        )
        .unwrap();
    let active_panel = match layout.node(group).unwrap() {
        Node::Tabs(tabs) => match layout.node(tabs.children[tabs.active_index]).unwrap() {
            Node::Panel(panel) => panel.panel_id,
            other => panic!("restored tab should point to a Panel node: {other:?}"),
        },
        other => panic!("remembered group should remain a Tabs node: {other:?}"),
    };
    assert_ne!(active_panel, closed_id);
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.validate(&metrics), Ok(()));

    let default_key = PanelKey::new("c43-default-placement").unwrap();
    let mut empty = Layout::new(Profile::Photoshop);
    empty
        .apply(
            DockCommand::OpenPanel {
                key: default_key.clone(),
                constraints: descriptors()[0].1,
                target: None,
            },
            &metrics,
        )
        .unwrap();
    let default_panel_id = empty
        .panel_instances()
        .find_map(|(id, instance)| (instance.key == default_key).then_some(id))
        .unwrap();
    empty
        .apply(
            DockCommand::ClosePanel {
                panel: default_panel_id,
            },
            &metrics,
        )
        .unwrap();
    assert!(empty.main_area(DockSide::Right).is_none());
    assert_eq!(
        empty.last_dock_location(&default_key),
        Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
    );

    // The former group/column no longer exists, so reopening must use the
    // right-dock profile fallback rather than a stale NodeId.
    empty
        .apply(
            DockCommand::OpenPanel {
                key: default_key,
                constraints: descriptors()[0].1,
                target: None,
            },
            &metrics,
        )
        .unwrap();
    assert!(empty.main_area(DockSide::Right).is_some());
    assert_eq!(empty.validate(&metrics), Ok(()));
}

#[test]
fn c43_opening_existing_singleton_activates_it_without_toggling_or_validating_new_target() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let singleton_key = PanelKey::new("c43-singleton").unwrap();
    let singleton_constraints =
        PanelConstraints::new(Size::new(80.0, 60.0), Size::new(180.0, 120.0), None, true);
    let (singleton_id, singleton_node) =
        builder.panel(singleton_key.clone(), singleton_constraints);
    let (_, other_node) = builder.panel(PanelKey::new("c43-other").unwrap(), descriptors()[0].1);
    let group = builder.tabs(vec![singleton_node, other_node], 1, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();

    let first = layout
        .apply(
            DockCommand::OpenPanel {
                key: singleton_key.clone(),
                constraints: singleton_constraints,
                target: Some(DropTarget::Tabs {
                    group: NodeId::null(),
                    index: 0,
                }),
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.panel_instances().count(), 2);
    assert!(layout.panel_instance(singleton_id).is_some());
    assert!(matches!(layout.node(group), Some(Node::Tabs(tabs)) if tabs.active_index == 0));
    assert_eq!(layout.version(), 1);
    assert!(first.inverse.is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));

    let second = layout
        .apply(
            DockCommand::OpenPanel {
                key: singleton_key,
                constraints: singleton_constraints,
                target: None,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.version(), 1);
    assert_eq!(second.new_version, 1);
    assert!(second.changed_nodes.is_empty());
    assert!(second.inverse.is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn moving_panel_to_own_group_edge_is_a_no_op() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (panel_id, panel_node) =
        builder.panel(PanelKey::new("own-edge-panel").unwrap(), descriptors()[0].1);
    let (_, sibling_node) = builder.panel(
        PanelKey::new("own-edge-sibling").unwrap(),
        descriptors()[0].1,
    );
    let group = builder.tabs(vec![panel_node, sibling_node], 0, true);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let before_version = layout.version();

    let outcome = layout
        .apply(
            DockCommand::MovePanel {
                panel: panel_id,
                target: DropTarget::GroupEdge {
                    group,
                    zone: crate::DropZone::Top,
                },
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.version(), before_version);
    assert!(outcome.changed_nodes.is_empty());
    assert!(
        matches!(layout.node(group), Some(Node::Tabs(tabs)) if tabs.collapsed && tabs.children == vec![panel_node,sibling_node])
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn resize_flyout_reports_the_core_clamped_width() {
    let metrics = DockMetrics::default();
    let constraints = descriptors()[0].1;
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (panel, panel_node) = builder.panel(
        PanelKey::new("resize-flyout-actual-width").unwrap(),
        constraints,
    );
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(
        metrics.rail_width,
        crate::ColumnMode::IconsOnly,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(DockCommand::OpenFlyout { column, panel }, &metrics)
        .unwrap();
    let outcome = layout
        .apply(DockCommand::ResizeFlyout { panel, width: 1.0 }, &metrics)
        .unwrap();

    assert_eq!(
        outcome.actual_value,
        Some(CommandValue::FlyoutWidth(constraints.min_size.width))
    );
    assert_eq!(
        layout.flyout_width(&PanelKey::new("resize-flyout-actual-width").unwrap()),
        Some(constraints.min_size.width)
    );
}
