use super::*;

#[test]
fn c46_float_panel_keeps_panel_instance_and_id() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel_id, moving_panel_node) = builder.panel(
        PanelKey::new("c46-float-panel").unwrap(),
        descriptors()[0].1,
    );
    let (_, sibling_panel_node) = builder.panel(
        PanelKey::new("c46-float-sibling").unwrap(),
        descriptors()[0].1,
    );
    let source_group = builder.tabs(vec![moving_panel_node, sibling_panel_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::Float {
                node: moving_panel_node,
                bounds: Rect::new(Point::new(80.0, 60.0), Size::new(320.0, 220.0)),
            },
            &metrics,
        )
        .unwrap();

    let (floating_id, floating) = layout.floating_groups().next().unwrap();
    let floating_root = layout.dock_area(floating.area).unwrap().root;
    assert!(matches!(
        layout.node(floating_root),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![moving_panel_node]
                && tabs.active_index == 0
                && !tabs.collapsed
    ));
    assert!(matches!(
        layout.node(moving_panel_node),
        Some(Node::Panel(panel)) if panel.panel_id == moving_panel_id
    ));
    assert!(layout.panel_instance(moving_panel_id).is_some());
    assert_eq!(
        layout.floating_group(floating_id).unwrap().bounds.size,
        Size::new(320.0, 220.0)
    );
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs)) if tabs.children == vec![sibling_panel_node]
    ));
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_float_entire_horizontal_column_composition_preserves_columns_and_groups() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, left_panel) = builder.panel(
        PanelKey::new("c46-float-composition-left").unwrap(),
        descriptors()[0].1,
    );
    let left_group = builder.tabs(vec![left_panel], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![left_group],
        vec![1.0],
    );

    let (_, right_panel) = builder.panel(
        PanelKey::new("c46-float-composition-right").unwrap(),
        descriptors()[0].1,
    );
    let right_group = builder.tabs(vec![right_panel], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![right_group],
        vec![1.0],
    );

    let composition = builder.split(Axis::X, vec![left_column, right_column], vec![0.4, 0.6]);
    builder.main_area(DockSide::Right, composition);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::Float {
                node: composition,
                bounds: Rect::new(Point::new(120.0, 80.0), Size::new(620.0, 640.0)),
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.main_area(DockSide::Right).is_none());
    let (floating_id, floating) = layout.floating_groups().next().unwrap();
    let floating_root = layout.dock_area(floating.area).unwrap().root;
    assert_eq!(floating_root, composition);
    assert!(matches!(
        layout.node(floating_root),
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children == vec![left_column, right_column]
                && split.ratios == vec![0.4, 0.6]
    ));
    assert_eq!(
        layout.floating_group(floating_id).unwrap().bounds,
        Rect::new(Point::new(120.0, 80.0), Size::new(620.0, 640.0))
    );
    assert!(
        matches!(layout.node(left_column), Some(Node::Column(column)) if column.groups == vec![left_group])
    );
    assert!(
        matches!(layout.node(right_column), Some(Node::Column(column)) if column.groups == vec![right_group])
    );
    assert_eq!(layout.validate(&metrics), Ok(()));

    // Closing one of the floating columns must normalize the composition to
    // the surviving column, rather than retaining the old horizontal split.
    layout
        .apply(DockCommand::CloseGroup { group: left_group }, &metrics)
        .unwrap();
    let floating = layout.floating_group(floating_id).unwrap();
    assert_eq!(layout.dock_area(floating.area).unwrap().root, right_column);
    assert!(layout.node(left_column).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_move_resize_raise_and_center_dock_floating_preserve_source_active_tab() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (dock_panel, dock_panel_node) = builder.panel(
        PanelKey::new("c46-center-dock").unwrap(),
        descriptors()[0].1,
    );
    let dock_group = builder.tabs(vec![dock_panel_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![dock_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);

    let (source_panel_a, source_node_a) =
        builder.panel(PanelKey::new("c46-floating-a").unwrap(), descriptors()[0].1);
    let (source_panel_b, source_node_b) =
        builder.panel(PanelKey::new("c46-floating-b").unwrap(), descriptors()[0].1);
    let source_tabs = builder.tabs(vec![source_node_a, source_node_b], 1, true);
    let source_floating = builder.floating_group(
        source_tabs,
        Rect::new(Point::new(20.0, 30.0), Size::new(420.0, 300.0)),
        1,
    );

    let (_, other_panel_node) = builder.panel(
        PanelKey::new("c46-other-floating").unwrap(),
        descriptors()[0].1,
    );
    let other_tabs = builder.tabs(vec![other_panel_node], 0, false);
    let other_floating = builder.floating_group(
        other_tabs,
        Rect::new(Point::new(50.0, 60.0), Size::new(360.0, 240.0)),
        2,
    );

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    let moved = layout
        .apply(
            DockCommand::MoveFloating {
                group: source_floating,
                origin: Point::new(140.0, 150.0),
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        layout
            .floating_group(source_floating)
            .unwrap()
            .bounds
            .origin,
        Point::new(140.0, 150.0)
    );
    layout.apply(moved.inverse.unwrap(), &metrics).unwrap();
    assert_eq!(
        layout
            .floating_group(source_floating)
            .unwrap()
            .bounds
            .origin,
        Point::new(20.0, 30.0)
    );

    let resized = layout
        .apply(
            DockCommand::ResizeFloating {
                group: source_floating,
                size: Size::new(10.0, 10.0),
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        resized.actual_value,
        Some(CommandValue::FloatingSize(Size::new(120.0, 80.0)))
    );
    assert_eq!(
        layout.floating_group(source_floating).unwrap().bounds.size,
        Size::new(120.0, 80.0)
    );
    layout.apply(resized.inverse.unwrap(), &metrics).unwrap();
    assert_eq!(
        layout.floating_group(source_floating).unwrap().bounds.size,
        Size::new(420.0, 300.0)
    );

    layout
        .apply(
            DockCommand::RaiseFloating {
                group: source_floating,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.floating_group(source_floating).unwrap().z_order, 3);
    assert_eq!(layout.floating_group(other_floating).unwrap().z_order, 2);
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: source_floating,
                target: DropTarget::Tabs {
                    group: dock_group,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();
    assert!(layout.floating_group(source_floating).is_none());
    assert!(layout.floating_group(other_floating).is_some());
    assert!(matches!(
        layout.node(dock_group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![dock_panel_node, source_node_a, source_node_b]
                && tabs.active_index == 2
    ));
    for panel in [dock_panel, source_panel_a, source_panel_b] {
        assert!(layout.panel_instance(panel).is_some());
    }
    assert_eq!(layout.panel_instances().count(), 4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_move_one_panel_between_groups_inside_floating_stack() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(
        PanelKey::new("c46-stack-moving").unwrap(),
        descriptors()[0].1,
    );
    let (remaining_panel, remaining_node) = builder.panel(
        PanelKey::new("c46-stack-remaining").unwrap(),
        descriptors()[0].1,
    );
    let source_group = builder.tabs(vec![moving_node, remaining_node], 0, false);
    let (target_panel, target_node) = builder.panel(
        PanelKey::new("c46-stack-target").unwrap(),
        descriptors()[0].1,
    );
    let target_group = builder.tabs(vec![target_node], 0, false);
    let stack = builder.split(Axis::Y, vec![source_group, target_group], vec![0.6, 0.4]);
    let floating_id = builder.floating_group(
        stack,
        Rect::new(Point::new(40.0, 50.0), Size::new(440.0, 360.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::Tabs {
                    group: target_group,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![remaining_node] && tabs.active_index == 0
    ));
    assert!(matches!(
        layout.node(target_group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![target_node, moving_node] && tabs.active_index == 1
    ));
    assert!(matches!(
        layout.node(moving_node),
        Some(Node::Panel(panel)) if panel.panel_id == moving_panel
    ));
    for panel in [moving_panel, remaining_panel, target_panel] {
        assert!(layout.panel_instance(panel).is_some());
    }
    let floating = layout.floating_group(floating_id).unwrap();
    assert!(matches!(
        layout.node(layout.dock_area(floating.area).unwrap().root),
        Some(Node::Split(split)) if split.axis == Axis::Y && split.children == vec![source_group, target_group]
    ));
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_move_whole_tabs_group_to_top_of_floating_stack_preserves_tabs_state() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel_a, moving_node_a) =
        builder.panel(PanelKey::new("c46-whole-a").unwrap(), descriptors()[0].1);
    let (moving_panel_b, moving_node_b) =
        builder.panel(PanelKey::new("c46-whole-b").unwrap(), descriptors()[0].1);
    let moving_group = builder.tabs(vec![moving_node_a, moving_node_b], 1, true);
    let main_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![moving_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, main_column);

    let (target_panel, target_node) = builder.panel(
        PanelKey::new("c46-stack-top-target").unwrap(),
        descriptors()[0].1,
    );
    let target_group = builder.tabs(vec![target_node], 0, false);
    let floating_id = builder.floating_group(
        target_group,
        Rect::new(Point::new(80.0, 90.0), Size::new(420.0, 320.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::SplitGroup {
                group: moving_group,
                target_group,
                zone: DropZone::Top,
                ratio: 0.3,
            },
            &metrics,
        )
        .unwrap();

    let floating = layout.floating_group(floating_id).unwrap();
    let root = layout.dock_area(floating.area).unwrap().root;
    assert!(matches!(
        layout.node(root),
        Some(Node::Split(split))
            if split.axis == Axis::Y
                && split.children == vec![moving_group, target_group]
                && split.ratios == vec![0.3, 0.7]
    ));
    assert!(matches!(
        layout.node(moving_group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![moving_node_a, moving_node_b]
                && tabs.active_index == 1
                && tabs.collapsed
    ));
    for panel in [moving_panel_a, moving_panel_b, target_panel] {
        assert!(layout.panel_instance(panel).is_some());
    }
    assert!(layout.main_area(DockSide::Left).is_none());
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));

    // Docking the whole floating stack creates a Column without flattening
    // the Tabs groups or losing their order and panel instances.
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
    assert!(layout.floating_group(floating_id).is_none());
    let docked_column = layout.main_area(DockSide::Left).unwrap().root;
    assert!(matches!(
        layout.node(docked_column),
        Some(Node::Column(column))
            if column.groups == vec![moving_group, target_group]
                && column.group_ratios.len() == 2
                && (column.group_ratios[0] - 0.3).abs() < 1.0e-4
                && (column.group_ratios[1] - 0.7).abs() < 1.0e-4
    ));
    assert!(matches!(
        layout.node(moving_group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![moving_node_a, moving_node_b]
                && tabs.active_index == 1
                && tabs.collapsed
    ));
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_photoshop_rejects_horizontal_split_inside_floating_stack_atomically() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, source_panel) = builder.panel(
        PanelKey::new("c46-horizontal-source").unwrap(),
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

    let (_, top_panel) = builder.panel(
        PanelKey::new("c46-horizontal-top").unwrap(),
        descriptors()[0].1,
    );
    let top_group = builder.tabs(vec![top_panel], 0, false);
    let (_, bottom_panel) = builder.panel(
        PanelKey::new("c46-horizontal-bottom").unwrap(),
        descriptors()[0].1,
    );
    let bottom_group = builder.tabs(vec![bottom_panel], 0, false);
    let stack = builder.split(Axis::Y, vec![top_group, bottom_group], vec![0.5, 0.5]);
    builder.floating_group(
        stack,
        Rect::new(Point::new(60.0, 70.0), Size::new(440.0, 340.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));
    let before = format!("{layout:?}");

    let error = layout
        .apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group: top_group,
                zone: DropZone::Left,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CommandError::InvalidTarget {
            reason: "horizontal splits are not supported inside a floating stack"
        }
    ));
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_successful_move_of_last_panel_cleans_up_floating_area() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (dock_panel, dock_panel_node) = builder.panel(
        PanelKey::new("c46-cleanup-dock").unwrap(),
        descriptors()[0].1,
    );
    let dock_group = builder.tabs(vec![dock_panel_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![dock_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);

    let (floating_panel, floating_panel_node) = builder.panel(
        PanelKey::new("c46-cleanup-floating").unwrap(),
        descriptors()[0].1,
    );
    let floating_root = builder.tabs(vec![floating_panel_node], 0, false);
    let floating_id = builder.floating_group(
        floating_root,
        Rect::new(Point::new(60.0, 70.0), Size::new(360.0, 260.0)),
        1,
    );
    let mut layout = builder.build();
    let area_id = layout.floating_group(floating_id).unwrap().area;
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::MovePanel {
                panel: floating_panel,
                target: DropTarget::Tabs {
                    group: dock_group,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(floating_id).is_none());
    assert!(layout.dock_area(area_id).is_none());
    assert!(layout.node(floating_root).is_none());
    assert!(matches!(
        layout.node(dock_group),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![dock_panel_node, floating_panel_node]
                && tabs.active_index == 1
    ));
    assert!(layout.panel_instance(floating_panel).is_some());
    assert!(layout.panel_instance(dock_panel).is_some());
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_failed_dock_preserves_the_floating_group_and_area_atomically() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, dock_panel_node) = builder.panel(
        PanelKey::new("c46-failed-dock-target").unwrap(),
        descriptors()[0].1,
    );
    let dock_group = builder.tabs(vec![dock_panel_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![dock_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let (floating_panel, floating_panel_node) = builder.panel(
        PanelKey::new("c46-failed-dock-source").unwrap(),
        descriptors()[0].1,
    );
    let floating_root = builder.tabs(vec![floating_panel_node], 0, false);
    let floating_id = builder.floating_group(
        floating_root,
        Rect::new(Point::new(90.0, 80.0), Size::new(420.0, 280.0)),
        2,
    );
    let mut layout = builder.build();
    let area_id = layout.floating_group(floating_id).unwrap().area;
    let before = format!("{layout:?}");
    assert_eq!(layout.validate(&metrics), Ok(()));

    let error = layout
        .apply(
            DockCommand::DockFloating {
                group: floating_id,
                target: DropTarget::Tabs {
                    group: NodeId::null(),
                    index: 0,
                },
            },
            &metrics,
        )
        .unwrap_err();

    assert!(matches!(error, CommandError::MissingNode { node } if node == NodeId::null()));
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), 0);
    assert!(layout.floating_group(floating_id).is_some());
    assert!(layout.dock_area(area_id).is_some());
    assert!(matches!(
        layout.node(floating_root),
        Some(Node::Tabs(tabs)) if tabs.children == vec![floating_panel_node]
    ));
    assert!(layout.panel_instance(floating_panel).is_some());
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_floating_bounds_are_model_space_and_are_not_screen_clamped() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) =
        builder.panel(PanelKey::new("c46-offscreen").unwrap(), descriptors()[0].1);
    let tabs = builder.tabs(vec![panel_node], 0, false);
    let floating_id = builder.floating_group(
        tabs,
        Rect::new(Point::new(-9000.0, -7000.0), Size::new(360.0, 260.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));
    assert_eq!(
        layout.floating_group(floating_id).unwrap().bounds.origin,
        Point::new(-9000.0, -7000.0)
    );

    let moved = layout
        .apply(
            DockCommand::MoveFloating {
                group: floating_id,
                origin: Point::new(-12000.0, 3000.0),
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(
        layout.floating_group(floating_id).unwrap().bounds.origin,
        Point::new(-12000.0, 3000.0)
    );
    layout.apply(moved.inverse.unwrap(), &metrics).unwrap();
    assert_eq!(
        layout.floating_group(floating_id).unwrap().bounds.origin,
        Point::new(-9000.0, -7000.0)
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_paint_and_hit_test_orders_are_inverse_views_of_z_order() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let mut ids = Vec::new();
    for (key, z_order) in [("c46-z-high", 9), ("c46-z-low", 1), ("c46-z-mid", 4)] {
        let (_, panel_node) = builder.panel(PanelKey::new(key).unwrap(), descriptors()[0].1);
        let tabs = builder.tabs(vec![panel_node], 0, false);
        ids.push(builder.floating_group(
            tabs,
            Rect::new(
                Point::new(20.0 * z_order as f32, 30.0),
                Size::new(300.0, 220.0),
            ),
            z_order,
        ));
    }
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));
    let [high, low, mid] = [ids[0], ids[1], ids[2]];

    assert_eq!(
        layout
            .floating_groups_in_paint_order()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![low, mid, high]
    );
    assert_eq!(
        layout
            .floating_groups_in_hit_test_order()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![high, mid, low]
    );

    layout
        .apply(DockCommand::RaiseFloating { group: low }, &metrics)
        .unwrap();
    assert_eq!(layout.floating_group(low).unwrap().z_order, 10);
    assert_eq!(
        layout
            .floating_groups_in_paint_order()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![mid, high, low]
    );
    assert_eq!(
        layout
            .floating_groups_in_hit_test_order()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![low, high, mid]
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn c46_center_merge_between_floating_windows_keeps_panel_instances_and_cleans_source() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (target_panel, target_node) = builder.panel(
        PanelKey::new("c46-floating-merge-target").unwrap(),
        descriptors()[0].1,
    );
    let target_tabs = builder.tabs(vec![target_node], 0, false);
    let target_floating = builder.floating_group(
        target_tabs,
        Rect::new(Point::new(100.0, 120.0), Size::new(420.0, 300.0)),
        1,
    );

    let (source_panel_a, source_node_a) = builder.panel(
        PanelKey::new("c46-floating-merge-a").unwrap(),
        descriptors()[0].1,
    );
    let (source_panel_b, source_node_b) = builder.panel(
        PanelKey::new("c46-floating-merge-b").unwrap(),
        descriptors()[0].1,
    );
    let source_tabs = builder.tabs(vec![source_node_a, source_node_b], 1, true);
    let source_floating = builder.floating_group(
        source_tabs,
        Rect::new(Point::new(160.0, 180.0), Size::new(400.0, 280.0)),
        2,
    );
    let mut layout = builder.build();
    let source_area = layout.floating_group(source_floating).unwrap().area;
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: source_floating,
                target: DropTarget::Tabs {
                    group: target_tabs,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(source_floating).is_none());
    assert!(layout.dock_area(source_area).is_none());
    assert!(layout.floating_group(target_floating).is_some());
    assert!(matches!(
        layout.node(target_tabs),
        Some(Node::Tabs(tabs))
            if tabs.children == vec![target_node, source_node_a, source_node_b]
                && tabs.active_index == 2
                && !tabs.collapsed
    ));
    for panel in [target_panel, source_panel_a, source_panel_b] {
        assert!(layout.panel_instance(panel).is_some());
    }
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_one_group_column_can_detach_from_horizontal_composition() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, selected_panel) = builder.panel(
        PanelKey::new("detach-column-selected").unwrap(),
        descriptors()[0].1,
    );
    let selected_group = builder.tabs(vec![selected_panel], 0, false);
    let selected_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![selected_group],
        vec![1.0],
    );

    let (_, sibling_panel) = builder.panel(
        PanelKey::new("detach-column-sibling").unwrap(),
        descriptors()[0].1,
    );
    let sibling_group = builder.tabs(vec![sibling_panel], 0, false);
    let sibling_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![sibling_group],
        vec![1.0],
    );

    let composition = builder.split(
        Axis::X,
        vec![selected_column, sibling_column],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, composition);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::Float {
                node: selected_column,
                bounds: Rect::new(Point::new(140.0, 90.0), Size::new(280.0, 420.0)),
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.validate(&metrics), Ok(()));
    assert_eq!(
        layout.main_area(DockSide::Right).map(|area| area.root),
        Some(sibling_column),
        "detaching a one-group column must leave its sibling docked"
    );
    let (_, floating) = layout
        .floating_groups()
        .next()
        .expect("detached column floats");
    assert_eq!(
        layout.dock_area(floating.area).map(|area| area.root),
        Some(selected_column),
        "the selected column itself must be the floating root"
    );
    assert!(matches!(
        layout.node(selected_column),
        Some(Node::Column(column)) if column.groups == vec![selected_group]
    ));
    assert!(matches!(
        layout.node(sibling_column),
        Some(Node::Column(column)) if column.groups == vec![sibling_group]
    ));
}

#[test]
fn docking_floating_stack_into_column_preserves_existing_group_ratio_proportions() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, main_a) = builder.panel(PanelKey::new("ratio-main-a").unwrap(), descriptors()[0].1);
    let (_, main_b) = builder.panel(PanelKey::new("ratio-main-b").unwrap(), descriptors()[0].1);
    let group_a = builder.tabs(vec![main_a], 0, false);
    let group_b = builder.tabs(vec![main_b], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![group_a, group_b],
        vec![0.8, 0.2],
    );
    builder.main_area(DockSide::Right, column);

    let (_, floating_a) = builder.panel(
        PanelKey::new("ratio-floating-a").unwrap(),
        descriptors()[0].1,
    );
    let (_, floating_b) = builder.panel(
        PanelKey::new("ratio-floating-b").unwrap(),
        descriptors()[0].1,
    );
    let floating_group_a = builder.tabs(vec![floating_a], 0, false);
    let floating_group_b = builder.tabs(vec![floating_b], 0, false);
    let stack = builder.split(
        Axis::Y,
        vec![floating_group_a, floating_group_b],
        vec![0.5, 0.5],
    );
    let floating = builder.floating_group(
        stack,
        Rect::new(Point::new(30.0, 40.0), Size::new(420.0, 320.0)),
        1,
    );
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::DockFloating {
                group: floating,
                target: DropTarget::ColumnSlot { column, index: 2 },
            },
            &metrics,
        )
        .unwrap();

    let Some(Node::Column(column_value)) = layout.node(column) else {
        panic!("main root should remain the target column");
    };
    assert_eq!(
        column_value.groups,
        vec![group_a, group_b, floating_group_a, floating_group_b]
    );
    assert_eq!(column_value.group_ratios.len(), 4);
    assert!((column_value.group_ratios[0] - 0.4).abs() < 1.0e-4);
    assert!((column_value.group_ratios[1] - 0.1).abs() < 1.0e-4);
    assert!((column_value.group_ratios[2] - 0.25).abs() < 1.0e-4);
    assert!((column_value.group_ratios[3] - 0.25).abs() < 1.0e-4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn docking_floating_group_onto_its_own_descendant_is_a_no_op() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, panel) = builder.panel(
        PanelKey::new("floating-self-drop").unwrap(),
        descriptors()[0].1,
    );
    let root = builder.tabs(vec![panel], 0, false);
    let floating = builder.floating_group(
        root,
        Rect::new(Point::new(20.0, 30.0), Size::new(400.0, 300.0)),
        1,
    );
    let mut layout = builder.build();
    let before_version = layout.version();

    let outcome = layout
        .apply(
            DockCommand::DockFloating {
                group: floating,
                target: DropTarget::Tabs {
                    group: root,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.version(), before_version);
    assert!(outcome.changed_nodes.is_empty());
    assert!(outcome.inverse.is_none());
    assert!(layout.floating_group(floating).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_geometry_commands_report_the_affected_root_for_ui_invalidation() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, main_panel) = builder.panel(
        PanelKey::new("changed-node-main").unwrap(),
        descriptors()[0].1,
    );
    let main_group = builder.tabs(vec![main_panel], 0, false);
    let main_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![main_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, main_column);

    let (_, floating_panel) = builder.panel(
        PanelKey::new("changed-node-floating").unwrap(),
        descriptors()[0].1,
    );
    let floating_root = builder.tabs(vec![floating_panel], 0, false);
    let floating = builder.floating_group(
        floating_root,
        Rect::new(Point::new(10.0, 20.0), Size::new(400.0, 300.0)),
        1,
    );
    let mut layout = builder.build();

    let moved = layout
        .apply(
            DockCommand::MoveFloating {
                group: floating,
                origin: Point::new(50.0, 60.0),
            },
            &metrics,
        )
        .unwrap();
    assert!(moved.changed_nodes.contains(&floating_root));

    let chrome = layout
        .apply(
            DockCommand::SetChrome {
                mode: crate::ChromeMode::DockHidden,
            },
            &metrics,
        )
        .unwrap();
    assert!(chrome.changed_nodes.contains(&main_column));
    assert!(chrome.changed_nodes.contains(&floating_root));
}

#[test]
fn floating_vertical_stack_reserves_title_row_when_first_group_collapsed() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, top_panel) = builder.panel(
        PanelKey::new("floating-title-collapsed").unwrap(),
        descriptors()[0].1,
    );
    let top_group = builder.tabs(vec![top_panel], 0, true);

    let (_, bottom_panel) = builder.panel(
        PanelKey::new("floating-title-expanded").unwrap(),
        descriptors()[0].1,
    );
    let bottom_group = builder.tabs(vec![bottom_panel], 0, false);

    let root = builder.split(Axis::Y, vec![top_group, bottom_group], vec![0.5, 0.5]);
    builder.floating_group(
        root,
        Rect::new(Point::new(20.0, 30.0), Size::new(440.0, 400.0)),
        1,
    );
    let layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    let rects = layout
        .compute_rects(Rect::new(Point::ZERO, Size::new(1200.0, 900.0)), &metrics)
        .unwrap();
    let top_rect = rects
        .groups
        .iter()
        .find(|entry| entry.group == top_group)
        .unwrap();
    assert_eq!(top_rect.rect.origin.y, 30.0 + metrics.header_height);
    assert!(rects.groups.iter().any(|entry| entry.group == bottom_group));
}

#[test]
fn dock_floating_columns_side_by_side_builds_a_single_horizontal_composition() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) = builder.panel(
        PanelKey::new("floating-compose-source").unwrap(),
        descriptors()[0].1,
    );
    let source_tabs = builder.tabs(vec![source_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_tabs],
        vec![1.0],
    );
    let source_floating = builder.floating_group(
        source_column,
        Rect::new(Point::new(40.0, 60.0), Size::new(300.0, 280.0)),
        2,
    );

    let (target_panel, target_node) = builder.panel(
        PanelKey::new("floating-compose-target").unwrap(),
        descriptors()[0].1,
    );
    let target_tabs = builder.tabs(vec![target_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![target_tabs],
        vec![1.0],
    );
    let target_floating = builder.floating_group(
        target_column,
        Rect::new(Point::new(420.0, 60.0), Size::new(420.0, 280.0)),
        1,
    );

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: source_floating,
                target: DropTarget::GroupEdge {
                    group: target_tabs,
                    zone: DropZone::Left,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(source_floating).is_none());
    let target = layout.floating_group(target_floating).unwrap();
    let root = layout.dock_area(target.area).unwrap().root;
    assert!(matches!(
        layout.node(root),
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children == vec![source_column, target_column]
                && (split.ratios[0] - 300.0 / 720.0).abs() < 1.0e-4
                && (split.ratios[1] - 420.0 / 720.0).abs() < 1.0e-4
    ));
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(column)) if column.groups == vec![source_tabs]
    ));
    assert!(matches!(
        layout.node(target_column),
        Some(Node::Column(column)) if column.groups == vec![target_tabs]
    ));
    assert!(layout.panel_instance(source_panel).is_some());
    assert!(layout.panel_instance(target_panel).is_some());
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn dock_floating_column_to_the_right_appends_after_the_existing_composition() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, left_panel) =
        builder.panel(PanelKey::new("floating-compose-left").unwrap(), descriptors()[0].1);
    let left_tabs = builder.tabs(vec![left_panel], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![left_tabs],
        vec![1.0],
    );
    let (_, right_panel) =
        builder.panel(PanelKey::new("floating-compose-right").unwrap(), descriptors()[0].1);
    let right_tabs = builder.tabs(vec![right_panel], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![right_tabs],
        vec![1.0],
    );
    let existing_composition = builder.split(
        Axis::X,
        vec![left_column, right_column],
        vec![0.4, 0.6],
    );
    let target_floating = builder.floating_group(
        existing_composition,
        Rect::new(Point::new(100.0, 80.0), Size::new(500.0, 300.0)),
        1,
    );

    let (_, source_panel) =
        builder.panel(PanelKey::new("floating-compose-new-right").unwrap(), descriptors()[0].1);
    let source_tabs = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_tabs],
        vec![1.0],
    );
    let source_floating = builder.floating_group(
        source_column,
        Rect::new(Point::new(620.0, 80.0), Size::new(280.0, 300.0)),
        2,
    );

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: source_floating,
                target: DropTarget::GroupEdge {
                    group: right_tabs,
                    zone: DropZone::Right,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(source_floating).is_none());
    let target = layout.floating_group(target_floating).unwrap();
    let root = layout.dock_area(target.area).unwrap().root;
    assert!(matches!(
        layout.node(root),
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children == vec![left_column, right_column, source_column]
                && split.ratios.len() == 3
                && (split.ratios.iter().sum::<f32>() - 1.0).abs() < 1.0e-4
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}



#[test]
fn docking_floating_horizontal_composition_to_empty_main_edge_preserves_the_tree() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, left_panel) =
        builder.panel(PanelKey::new("dock-empty-main-left").unwrap(), descriptors()[0].1);
    let left_group = builder.tabs(vec![left_panel], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![left_group],
        vec![1.0],
    );

    let (_, right_panel) =
        builder.panel(PanelKey::new("dock-empty-main-right").unwrap(), descriptors()[0].1);
    let right_group = builder.tabs(vec![right_panel], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![right_group],
        vec![1.0],
    );

    let composition = builder.split(
        Axis::X,
        vec![left_column, right_column],
        vec![0.35, 0.65],
    );
    let floating = builder.floating_group(
        composition,
        Rect::new(Point::new(40.0, 60.0), Size::new(560.0, 320.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: floating,
                target: DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(floating).is_none());
    assert_eq!(
        layout.main_area(DockSide::Left).map(|area| area.root),
        Some(composition)
    );
    assert!(matches!(
        layout.node(composition),
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children == vec![left_column, right_column]
                && split.ratios == vec![0.35, 0.65]
    ));
    assert!(layout.main_area(DockSide::Right).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn docking_floating_horizontal_composition_before_existing_main_column_keeps_all_columns() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, main_panel) =
        builder.panel(PanelKey::new("dock-existing-main").unwrap(), descriptors()[0].1);
    let main_group = builder.tabs(vec![main_panel], 0, false);
    let main_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![main_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, main_column);

    let (_, source_left_panel) =
        builder.panel(PanelKey::new("dock-existing-source-left").unwrap(), descriptors()[0].1);
    let source_left_group = builder.tabs(vec![source_left_panel], 0, false);
    let source_left_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_left_group],
        vec![1.0],
    );

    let (_, source_right_panel) =
        builder.panel(PanelKey::new("dock-existing-source-right").unwrap(), descriptors()[0].1);
    let source_right_group = builder.tabs(vec![source_right_panel], 0, false);
    let source_right_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![source_right_group],
        vec![1.0],
    );

    let composition = builder.split(
        Axis::X,
        vec![source_left_column, source_right_column],
        vec![0.4, 0.6],
    );
    let floating = builder.floating_group(
        composition,
        Rect::new(Point::new(400.0, 80.0), Size::new(560.0, 320.0)),
        1,
    );
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: floating,
                target: DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(floating).is_none());
    let main_root = layout.main_area(DockSide::Left).unwrap().root;
    assert!(matches!(
        layout.node(main_root),
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children == vec![source_left_column, source_right_column, main_column]
                && split.ratios.len() == 3
                && (split.ratios[0] - (0.4 * 2.0 / 3.0)).abs() < 1.0e-4
                && (split.ratios[1] - (0.6 * 2.0 / 3.0)).abs() < 1.0e-4
                && (split.ratios[2] - (1.0 / 3.0)).abs() < 1.0e-4
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}
