use super::*;

#[test]
fn normalize_migrates_legacy_floating_column_to_vertical_tabs_stack() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
    let (_, first_panel) =
        builder.panel(PanelKey::new("legacy-floating-column-a").unwrap(), descriptors()[0].1);
    let first_group = builder.tabs(vec![first_panel], 0, false);
    let (_, second_panel) =
        builder.panel(PanelKey::new("legacy-floating-column-b").unwrap(), descriptors()[0].1);
    let second_group = builder.tabs(vec![second_panel], 0, true);
    let legacy_root = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![first_group, second_group],
        vec![0.3, 0.7],
    );
    let floating = builder.floating_group(
        legacy_root,
        Rect::new(Point::new(50.0, 60.0), Size::new(400.0, 300.0)),
        1,
    );
    let mut layout = builder.build();

    assert!(layout.validate(&metrics).is_err());
    layout.normalize_for_test(&metrics).unwrap();

    let area = layout.floating_group(floating).unwrap().area;
    let migrated_root = layout.dock_area(area).unwrap().root;
    assert!(matches!(
        layout.node(migrated_root),
        Some(Node::Split(split))
            if split.axis == Axis::Y
                && split.children == vec![first_group, second_group]
                && split.ratios == vec![0.3, 0.7]
    ));
    assert!(layout.node(legacy_root).is_none());
    assert!(matches!(layout.node(first_group), Some(Node::Tabs(_))));
    assert!(matches!(layout.node(second_group), Some(Node::Tabs(_))));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn normalize_migrates_legacy_horizontal_floating_composition_to_vertical_stack() {
    let metrics = DockMetrics::default();
    let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

    let (_, left_panel) =
        builder.panel(PanelKey::new("legacy-floating-horizontal-left").unwrap(), descriptors()[0].1);
    let left_group = builder.tabs(vec![left_panel], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![left_group],
        vec![1.0],
    );

    let (_, right_a) =
        builder.panel(PanelKey::new("legacy-floating-horizontal-right-a").unwrap(), descriptors()[0].1);
    let (_, right_b) =
        builder.panel(PanelKey::new("legacy-floating-horizontal-right-b").unwrap(), descriptors()[0].1);
    let right_group_a = builder.tabs(vec![right_a], 0, false);
    let right_group_b = builder.tabs(vec![right_b], 0, true);
    let right_column = builder.column(
        metrics.default_column_width,
        crate::ColumnMode::Expanded,
        vec![right_group_a, right_group_b],
        vec![0.25, 0.75],
    );

    let legacy_root = builder.split(
        Axis::X,
        vec![left_column, right_column],
        vec![0.4, 0.6],
    );
    let floating = builder.floating_group(
        legacy_root,
        Rect::new(Point::new(70.0, 80.0), Size::new(640.0, 360.0)),
        1,
    );
    let mut layout = builder.build();

    assert!(layout.validate(&metrics).is_err());
    layout.normalize_for_test(&metrics).unwrap();

    let area = layout.floating_group(floating).unwrap().area;
    let migrated_root = layout.dock_area(area).unwrap().root;
    assert!(matches!(
        layout.node(migrated_root),
        Some(Node::Split(split))
            if split.axis == Axis::Y
                && split.children == vec![left_group, right_group_a, right_group_b]
                && split.ratios.len() == 3
                && (split.ratios[0] - 0.4).abs() < 1.0e-4
                && (split.ratios[1] - 0.15).abs() < 1.0e-4
                && (split.ratios[2] - 0.45).abs() < 1.0e-4
    ));
    assert!(layout.node(legacy_root).is_none());
    assert!(layout.node(left_column).is_none());
    assert!(layout.node(right_column).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn normalize_removes_an_empty_main_root_and_area() {
    let mut nodes = SlotMap::<NodeId, Node>::with_key();
    let root = nodes.insert(Node::Tabs(Tabs {
        children: Vec::new(),
        active_index: 0,
        collapsed: false,
    }));
    let panels = SlotMap::<PanelId, PanelInstance>::with_key();
    let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
    let area = areas.insert(DockArea::main(DockSide::Right, root));

    let mut layout = Layout::from_parts(
        nodes,
        panels,
        areas,
        [None, Some(area)],
        SlotMap::<FloatingGroupId, FloatingGroup>::with_key(),
        CanvasState::default(),
        Profile::Generic,
        0,
        ChromeMode::Visible,
    );

    layout.normalize(&DockMetrics::default()).unwrap();
    assert_eq!(layout.nodes().count(), 0);
    assert_eq!(layout.dock_areas().count(), 0);
    assert!(layout.main_area(DockSide::Right).is_none());
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));

    // Running normalization again is safe and does not recreate an area.
    layout.normalize(&DockMetrics::default()).unwrap();
    assert_eq!(layout.nodes().count(), 0);
    assert_eq!(layout.dock_areas().count(), 0);
}

#[test]
fn normalize_collapses_a_generic_split_with_one_child() {
    let mut panels = SlotMap::<PanelId, PanelInstance>::with_key();
    let panel_id = panels.insert(PanelInstance::new(
        PanelKey::new("layers").unwrap(),
        descriptors()[0].1,
    ));
    let mut nodes = SlotMap::<NodeId, Node>::with_key();
    let panel_node = nodes.insert(Node::Panel(Panel::new(panel_id)));
    let tabs_node = nodes.insert(Node::Tabs(Tabs {
        children: vec![panel_node],
        active_index: 0,
        collapsed: false,
    }));
    let split_node = nodes.insert(Node::Split(Split {
        axis: Axis::Y,
        children: vec![tabs_node],
        ratios: vec![1.0],
    }));
    let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
    let area = areas.insert(DockArea::main(DockSide::Right, split_node));

    let mut layout = Layout::from_parts(
        nodes,
        panels,
        areas,
        [None, Some(area)],
        SlotMap::<FloatingGroupId, FloatingGroup>::with_key(),
        CanvasState::default(),
        Profile::Generic,
        0,
        ChromeMode::Visible,
    );

    layout.normalize(&DockMetrics::default()).unwrap();
    assert!(layout.node(split_node).is_none());
    assert!(matches!(layout.node(tabs_node), Some(Node::Tabs(_))));
    assert_eq!(layout.main_area(DockSide::Right).unwrap().root, tabs_node);
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
}

#[test]
fn normalize_removes_empty_groups_but_preserves_single_group_columns() {
    let mut layout = initial_layout(Profile::Generic);
    let root = layout.main_area(DockSide::Right).unwrap().root;
    let groups = match layout.node(root).unwrap() {
        Node::Column(column) => column.groups.clone(),
        _ => panic!("factory should create a Column"),
    };
    let empty_group = groups[1];
    let panel_nodes = match layout.node(empty_group).unwrap() {
        Node::Tabs(tabs) => tabs.children.clone(),
        _ => panic!("Column groups should be Tabs"),
    };

    let panel_ids: Vec<_> = panel_nodes
        .iter()
        .filter_map(|node_id| match layout.node(*node_id).unwrap() {
            Node::Panel(panel) => Some((*node_id, panel.panel_id)),
            _ => None,
        })
        .collect();
    for (node_id, panel_id) in panel_ids {
        layout.nodes.remove(node_id);
        layout.panel_instances.remove(panel_id);
    }
    let Node::Tabs(tabs) = layout.nodes.get_mut(empty_group).unwrap() else {
        unreachable!();
    };
    tabs.children.clear();
    tabs.active_index = 0;

    let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
        unreachable!();
    };
    column.group_ratios = vec![0.2, 0.2];

    layout.normalize(&DockMetrics::default()).unwrap();

    let Node::Column(column) = layout.node(root).unwrap() else {
        panic!("Column must not collapse when only one group remains");
    };
    assert_eq!(column.groups, vec![groups[0]]);
    assert_eq!(column.group_ratios, vec![1.0]);
    assert!(layout.node(empty_group).is_none());
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));

    let node_count = layout.nodes().count();
    layout.normalize(&DockMetrics::default()).unwrap();
    assert_eq!(layout.nodes().count(), node_count);
    let Node::Column(column) = layout.node(root).unwrap() else {
        panic!("Column must remain a Column");
    };
    assert_eq!(column.groups, vec![groups[0]]);
    assert_eq!(column.group_ratios, vec![1.0]);
}

#[test]
fn normalize_repairs_ratios_and_clamps_active_tab_index_idempotently() {
    let mut layout = initial_layout(Profile::Generic);
    let root = layout.main_area(DockSide::Right).unwrap().root;
    let group_id = match layout.node(root).unwrap() {
        Node::Column(column) => column.groups[0],
        _ => unreachable!(),
    };
    let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
        unreachable!();
    };
    column.group_ratios = vec![0.2, 0.2];
    let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
        unreachable!();
    };
    tabs.active_index = 99;

    layout.normalize(&DockMetrics::default()).unwrap();
    let Node::Column(column) = layout.node(root).unwrap() else {
        unreachable!();
    };
    assert_eq!(column.group_ratios, vec![0.5, 0.5]);
    let Node::Tabs(tabs) = layout.node(group_id).unwrap() else {
        unreachable!();
    };
    assert_eq!(tabs.active_index, 1);

    layout.normalize(&DockMetrics::default()).unwrap();
    let Node::Column(column) = layout.node(root).unwrap() else {
        unreachable!();
    };
    assert_eq!(column.group_ratios, vec![0.5, 0.5]);
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
}

#[test]
fn normalize_refuses_to_hide_missing_links_or_cycles_and_is_transactional() {
    let mut broken_link = initial_layout(Profile::Generic);
    let root = broken_link.main_area(DockSide::Right).unwrap().root;
    let node_count = broken_link.nodes().count();
    let Node::Column(column) = broken_link.nodes.get_mut(root).unwrap() else {
        unreachable!();
    };
    column.groups.push(NodeId::null());
    column.group_ratios.push(0.0);

    let errors = broken_link.normalize(&DockMetrics::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, ValidationError::MissingNode { .. }))
    );
    assert_eq!(broken_link.nodes().count(), node_count);
    let Node::Column(column) = broken_link.node(root).unwrap() else {
        unreachable!();
    };
    assert_eq!(column.groups.last().copied(), Some(NodeId::null()));

    let mut cycle = initial_layout(Profile::Generic);
    let root = cycle.main_area(DockSide::Right).unwrap().root;
    let Node::Column(column) = cycle.nodes.get_mut(root).unwrap() else {
        unreachable!();
    };
    column.groups.push(root);
    column.group_ratios.push(0.0);

    let errors = cycle.normalize(&DockMetrics::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, ValidationError::NodeCycle { .. }))
    );
    let Node::Column(column) = cycle.node(root).unwrap() else {
        unreachable!();
    };
    assert_eq!(column.groups.last().copied(), Some(root));
}

#[test]
fn normalize_removes_an_empty_column_and_its_main_area() {
    let mut nodes = SlotMap::<NodeId, Node>::with_key();
    let root = nodes.insert(Node::Column(crate::Column {
        width: DockMetrics::default().default_column_width,
        mode: crate::ColumnMode::Expanded,
        groups: Vec::new(),
        group_ratios: Vec::new(),
        last_expanded_width: Some(DockMetrics::default().default_column_width),
    }));
    let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
    let area = areas.insert(DockArea::main(DockSide::Right, root));
    let mut layout = Layout::from_parts(
        nodes,
        SlotMap::<PanelId, PanelInstance>::with_key(),
        areas,
        [None, Some(area)],
        SlotMap::<FloatingGroupId, FloatingGroup>::with_key(),
        CanvasState::default(),
        Profile::Photoshop,
        0,
        ChromeMode::Visible,
    );

    layout.normalize(&DockMetrics::default()).unwrap();
    assert!(layout.main_area(DockSide::Right).is_none());
    assert_eq!(layout.nodes().count(), 0);
    assert_eq!(layout.dock_areas().count(), 0);
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
}

#[test]
fn normalize_removes_empty_floating_root_group_and_area() {
    let mut nodes = SlotMap::<NodeId, Node>::with_key();
    let root = nodes.insert(Node::Tabs(Tabs {
        children: Vec::new(),
        active_index: 0,
        collapsed: false,
    }));
    let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
    let area = areas.insert(DockArea::floating(root));
    let mut floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
    let group = floating.insert(FloatingGroup::new(
        area,
        Rect::new(Point::new(40.0, 50.0), Size::new(400.0, 300.0)),
        1,
    ));
    let mut layout = Layout::from_parts(
        nodes,
        SlotMap::<PanelId, PanelInstance>::with_key(),
        areas,
        [None, None],
        floating,
        CanvasState::default(),
        Profile::Generic,
        0,
        ChromeMode::Visible,
    );

    layout.normalize(&DockMetrics::default()).unwrap();
    assert!(layout.floating_group(group).is_none());
    assert_eq!(layout.nodes().count(), 0);
    assert_eq!(layout.dock_areas().count(), 0);
    assert_eq!(layout.floating_groups().count(), 0);
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
}

#[test]
fn normalization_eligibility_uses_typed_validation_reasons() {
    use crate::ValidationReason;

    let node = NodeId::null();
    let is_normalizable = super::super::helpers::is_normalizable_validation_error;

    assert_eq!(
        ValidationError::InvalidColumn {
            node,
            reason: ValidationReason::RatiosMustSumToOne,
        }
        .to_string(),
        format!("invalid Column node {node:?}: ratios must sum to 1.0 within 1e-4")
    );

    assert!(is_normalizable(&ValidationError::InvalidSplit {
        node,
        reason: ValidationReason::SplitMustContainAtLeastTwoChildren,
    }));
    assert!(is_normalizable(&ValidationError::InvalidSplit {
        node,
        reason: ValidationReason::RatiosMustSumToOne,
    }));
    assert!(is_normalizable(&ValidationError::InvalidTabs {
        node,
        reason: ValidationReason::TabsMustContainAtLeastOneChild,
    }));
    assert!(is_normalizable(&ValidationError::InvalidTabs {
        node,
        reason: ValidationReason::ActiveTabIndexOutsideChildren,
    }));
    assert!(is_normalizable(&ValidationError::InvalidColumn {
        node,
        reason: ValidationReason::ColumnMustContainAtLeastOneGroupBeforeNormalization,
    }));
    assert!(is_normalizable(&ValidationError::InvalidColumn {
        node,
        reason: ValidationReason::RatioCountMustMatchChildGroupCount,
    }));

    assert!(!is_normalizable(&ValidationError::InvalidTabs {
        node,
        reason: ValidationReason::PhotoshopTabsChildrenMustBePanelNodes,
    }));
    assert!(!is_normalizable(&ValidationError::InvalidColumn {
        node,
        reason: ValidationReason::ColumnWidthMustBeFinite,
    }));
}
