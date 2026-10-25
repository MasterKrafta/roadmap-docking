use super::*;

#[test]
fn c44_horizontal_split_creates_a_neighbor_column_with_semantic_order() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(
        PanelKey::new("c44-horizontal-source").unwrap(),
        descriptors()[0].1,
    );
    let source_group = builder.tabs(vec![source_panel], 0, true);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(
        PanelKey::new("c44-horizontal-target").unwrap(),
        descriptors()[0].1,
    );
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
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
                ratio: 0.25,
            },
            &metrics,
        )
        .unwrap();

    let root = layout.main_area(DockSide::Right).unwrap().root;
    let Node::Split(split) = layout.node(root).unwrap() else {
        panic!("horizontal side drop should produce Split(X)");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 2);
    assert_eq!(split.children[0], target_column);
    let new_column = split.children[1];
    assert!(matches!(
        layout.node(new_column),
        Some(Node::Column(column)) if column.groups == vec![source_group]
    ));
    let target_width = metrics.default_column_width;
    let new_width = metrics.min_column_width;
    let total_width = target_width + new_width;
    assert!((split.ratios[0] - target_width / total_width).abs() < 1.0e-5);
    assert!((split.ratios[1] - new_width / total_width).abs() < 1.0e-5);
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs)) if tabs.children == vec![source_panel] && tabs.collapsed
    ));
    assert!(layout.main_area(DockSide::Left).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c44_group_move_preserves_tabs_state_across_column_floating_and_docking() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (panel_a, node_a) =
        builder.panel(PanelKey::new("c44-panel-a").unwrap(), descriptors()[0].1);
    let (panel_b, node_b) =
        builder.panel(PanelKey::new("c44-panel-b").unwrap(), descriptors()[0].1);
    let (panel_c, node_c) =
        builder.panel(PanelKey::new("c44-panel-c").unwrap(), descriptors()[0].1);
    let source_group = builder.tabs(vec![node_a, node_b, node_c], 1, true);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (target_panel, target_node) =
        builder.panel(PanelKey::new("c44-target").unwrap(), descriptors()[0].1);
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));
    let expected_children = vec![node_a, node_b, node_c];

    layout
        .apply(
            DockCommand::MoveGroup {
                group: source_group,
                column: target_column,
                index: 1,
            },
            &metrics,
        )
        .unwrap();
    assert!(layout.main_area(DockSide::Left).is_none());
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs))
            if tabs.children == expected_children && tabs.active_index == 1 && tabs.collapsed
    ));
    assert!(matches!(
        layout.node(target_column),
        Some(Node::Column(column)) if column.groups == vec![target_group, source_group]
    ));
    for panel in [panel_a, panel_b, panel_c, target_panel] {
        assert!(layout.panel_instance(panel).is_some());
    }
    assert_eq!(layout.panel_instances().count(), 4);
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::Float {
                node: source_group,
                bounds: Rect::new(Point::new(70.0, 60.0), Size::new(420.0, 300.0)),
            },
            &metrics,
        )
        .unwrap();
    let floating_id = layout.floating_groups().next().unwrap().0;
    let floating_root = layout
        .dock_area(layout.floating_group(floating_id).unwrap().area)
        .unwrap()
        .root;
    assert_eq!(floating_root, source_group);
    assert!(matches!(
        layout.node(floating_root),
        Some(Node::Tabs(tabs))
            if tabs.children == expected_children && tabs.active_index == 1 && tabs.collapsed
    ));
    assert_eq!(layout.panel_instances().count(), 4);
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: floating_id,
                target: DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                },
            },
            &metrics,
        )
        .unwrap();
    let redocked_column = layout.main_area(DockSide::Left).unwrap().root;
    assert!(matches!(
        layout.node(redocked_column),
        Some(Node::Column(column)) if column.groups == vec![source_group]
    ));
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs))
            if tabs.children == expected_children && tabs.active_index == 1 && tabs.collapsed
    ));
    for panel in [panel_a, panel_b, panel_c, target_panel] {
        assert!(layout.panel_instance(panel).is_some());
    }
    assert_eq!(layout.panel_instances().count(), 4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c44_drop_on_existing_neighbor_column_is_a_noop() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, source_panel) = builder.panel(
        PanelKey::new("c44-noop-source").unwrap(),
        descriptors()[0].1,
    );
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    let (_, target_panel) = builder.panel(
        PanelKey::new("c44-noop-target").unwrap(),
        descriptors()[0].1,
    );
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    let root = builder.split(
        Axis::X,
        vec![source_column, target_column],
        vec![0.35, 0.65],
    );
    builder.main_area(DockSide::Right, root);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    let outcome = layout
        .apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group,
                zone: DropZone::Left,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(outcome.new_version, 0);
    assert!(outcome.changed_nodes.is_empty());
    assert!(outcome.inverse.is_none());
    assert_eq!(layout.version(), 0);
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c44_move_group_and_vertical_split_to_existing_neighbor_are_noops() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();

    for index in 0..3 {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("c44-neighbor-noop-{index}")).unwrap(),
            descriptors()[0].1,
        );
        groups.push(builder.tabs(vec![panel], 0, false));
    }
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        groups.clone(),
        vec![0.2, 0.3, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let before = format!("{layout:?}");

    let same_slot = layout
        .apply(
            DockCommand::MoveGroup {
                group: groups[0],
                column,
                index: 1,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(same_slot.new_version, 0);
    assert!(same_slot.changed_nodes.is_empty());
    assert!(same_slot.inverse.is_none());
    assert_eq!(format!("{layout:?}"), before);

    let above_neighbor = layout
        .apply(
            DockCommand::SplitGroup {
                group: groups[0],
                target_group: groups[1],
                zone: DropZone::Top,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(above_neighbor.new_version, 0);
    assert!(above_neighbor.changed_nodes.is_empty());
    assert!(above_neighbor.inverse.is_none());
    assert_eq!(format!("{layout:?}"), before);

    let below_neighbor = layout
        .apply(
            DockCommand::SplitGroup {
                group: groups[1],
                target_group: groups[0],
                zone: DropZone::Bottom,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(below_neighbor.new_version, 0);
    assert!(below_neighbor.changed_nodes.is_empty());
    assert!(below_neighbor.inverse.is_none());
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c44_split_group_moves_whole_tabs_into_an_existing_floating_stack() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (panel_a, node_a) = builder.panel(
        PanelKey::new("c44-floating-source-a").unwrap(),
        descriptors()[0].1,
    );
    let (panel_b, node_b) = builder.panel(
        PanelKey::new("c44-floating-source-b").unwrap(),
        descriptors()[0].1,
    );
    let source_group = builder.tabs(vec![node_a, node_b], 1, true);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (target_panel, target_node) = builder.panel(
        PanelKey::new("c44-floating-target").unwrap(),
        descriptors()[0].1,
    );
    let target_group = builder.tabs(vec![target_node], 0, false);
    let floating_id = builder.floating_group(
        target_group,
        Rect::new(Point::new(60.0, 70.0), Size::new(400.0, 280.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group,
                zone: DropZone::Bottom,
                ratio: 0.35,
            },
            &metrics,
        )
        .unwrap();

    let floating = layout.floating_group(floating_id).unwrap();
    let root = layout.dock_area(floating.area).unwrap().root;
    let Node::Split(split) = layout.node(root).unwrap() else {
        panic!("dropping below the floating root should create Split(Y)");
    };
    assert_eq!(split.axis, Axis::Y);
    assert_eq!(split.children, vec![target_group, source_group]);
    assert_eq!(split.ratios, vec![0.65, 0.35]);
    assert!(layout.main_area(DockSide::Left).is_none());
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![node_a, node_b]
                && tabs.active_index == 1
                && tabs.collapsed
    ));
    for panel in [panel_a, panel_b, target_panel] {
        assert!(layout.panel_instance(panel).is_some());
    }
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c44_drop_on_adjacent_floating_stack_slot_is_a_noop() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, top_panel) = builder.panel(
        PanelKey::new("c44-floating-noop-top").unwrap(),
        descriptors()[0].1,
    );
    let top_group = builder.tabs(vec![top_panel], 0, false);
    let (_, bottom_panel) = builder.panel(
        PanelKey::new("c44-floating-noop-bottom").unwrap(),
        descriptors()[0].1,
    );
    let bottom_group = builder.tabs(vec![bottom_panel], 0, false);
    let stack = builder.split(Axis::Y, vec![top_group, bottom_group], vec![0.4, 0.6]);
    builder.floating_group(
        stack,
        Rect::new(Point::new(40.0, 60.0), Size::new(380.0, 260.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));
    let before = format!("{layout:?}");

    let outcome = layout
        .apply(
            DockCommand::SplitGroup {
                group: top_group,
                target_group: bottom_group,
                zone: DropZone::Top,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(outcome.new_version, 0);
    assert!(outcome.changed_nodes.is_empty());
    assert!(outcome.inverse.is_none());
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c44_max_column_limit_allows_group_move_when_empty_source_column_collapses() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    let mut columns = Vec::new();
    let mut panel_ids = Vec::new();

    for index in 0..metrics.max_columns {
        let (panel_id, panel) = builder.panel(
            PanelKey::new(format!("c44-cap-panel-{index}")).unwrap(),
            descriptors()[0].1,
        );
        let group = builder.tabs(vec![panel], 0, index == 0);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        groups.push(group);
        columns.push(column);
        panel_ids.push(panel_id);
    }
    let root = builder.split(
        Axis::X,
        columns.clone(),
        vec![1.0 / metrics.max_columns as f32; metrics.max_columns],
    );
    builder.main_area(DockSide::Right, root);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::SplitGroup {
                group: groups[0],
                target_group: groups[2],
                zone: DropZone::Right,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap();

    let root = layout.main_area(DockSide::Right).unwrap().root;
    let Node::Split(split) = layout.node(root).unwrap() else {
        panic!("main dock should remain a horizontal split");
    };
    assert_eq!(split.children.len(), metrics.max_columns);
    assert!(!split.children.contains(&columns[0]));
    assert!(split.children.iter().any(|column_id| {
        matches!(layout.node(*column_id), Some(Node::Column(column)) if column.groups == vec![groups[0]])
    }));
    assert!(matches!(
        layout.node(groups[0]),
        Some(Node::Tabs(tabs)) if tabs.children.len() == 1 && tabs.collapsed
    ));
    assert_eq!(layout.panel_instances().count(), metrics.max_columns);
    for panel in panel_ids {
        assert!(layout.panel_instance(panel).is_some());
    }
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c44_stale_horizontal_drop_at_column_limit_is_rejected_atomically() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut columns = Vec::new();
    let mut target_groups = Vec::new();

    let (_, source_panel) = builder.panel(
        PanelKey::new("c44-stale-source").unwrap(),
        descriptors()[0].1,
    );
    let source_group = builder.tabs(vec![source_panel], 0, false);

    let (_, sibling_panel) = builder.panel(
        PanelKey::new("c44-stale-sibling").unwrap(),
        descriptors()[0].1,
    );
    let sibling_group = builder.tabs(vec![sibling_panel], 0, false);
    let first_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group, sibling_group],
        vec![0.5, 0.5],
    );
    columns.push(first_column);

    for index in 1..metrics.max_columns {
        let (_, panel) = builder.panel(
            PanelKey::new(format!("c44-full-panel-{index}")).unwrap(),
            descriptors()[0].1,
        );
        let group = builder.tabs(vec![panel], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        target_groups.push(group);
        columns.push(column);
    }

    let root = builder.split(
        Axis::X,
        columns.clone(),
        vec![1.0 / metrics.max_columns as f32; metrics.max_columns],
    );
    builder.main_area(DockSide::Right, root);

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));
    let before = format!("{layout:?}");

    // The source column retains sibling_group, so this horizontal move
    // would leave max_columns + 1 nonempty columns in the final layout.
    let error = layout
        .apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group: target_groups[0],
                zone: DropZone::Left,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CommandError::InvalidLayout { errors }
            if errors.iter().any(|error| matches!(
                error,
                ValidationError::TooManyColumns { actual, max }
                    if *actual > *max
            ))
    ));
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);
    assert_eq!(layout.validate(&metrics), Ok(()));
}
