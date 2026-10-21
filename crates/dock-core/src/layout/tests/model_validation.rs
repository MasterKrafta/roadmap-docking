use super::*;

#[test]
fn new_layout_owns_empty_arenas_and_an_independent_canvas() {
    let layout = Layout::new(Profile::Generic);

    assert_eq!(layout.profile(), Profile::Generic);
    assert_eq!(layout.version(), 0);
    assert_eq!(layout.chrome(), ChromeMode::Visible);
    assert_eq!(layout.flyout(), None);
    assert_eq!(layout.canvas(), &CanvasState::default());
    assert_eq!(layout.nodes().count(), 0);
    assert_eq!(layout.panel_instances().count(), 0);
    assert_eq!(layout.dock_areas().count(), 0);
    assert_eq!(layout.floating_groups().count(), 0);
    assert_eq!(layout.main_area(DockSide::Left), None);
    assert_eq!(layout.main_area(DockSide::Right), None);
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
}

#[test]
fn clone_preserves_runtime_handles_and_read_only_views() {
    let mut nodes = SlotMap::<NodeId, Node>::with_key();
    let root = nodes.insert(Node::Split(Split {
        axis: Axis::Y,
        children: Vec::new(),
        ratios: Vec::new(),
    }));
    let panels = SlotMap::<PanelId, PanelInstance>::with_key();
    let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
    let area = areas.insert(DockArea::main(DockSide::Left, root));
    let floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
    let layout = Layout::from_parts(
        nodes,
        panels,
        areas,
        [Some(area), None],
        floating,
        CanvasState::default(),
        Profile::Generic,
        7,
        ChromeMode::AllHidden,
    );
    let cloned = layout.clone();

    assert_eq!(cloned.main_area(DockSide::Left).unwrap().root, root);
    assert_eq!(cloned.version(), 7);
    assert_eq!(cloned.chrome(), ChromeMode::AllHidden);
}

#[test]
fn initial_factory_layout_passes_validation() {
    let layout = initial_layout(Profile::Photoshop);
    assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
}

#[test]
fn validation_collects_ratio_and_tabs_errors() {
    let mut layout = initial_layout(Profile::Generic);
    let root = layout.main_area(DockSide::Right).unwrap().root;
    let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
        panic!("factory should create a column");
    };
    column.group_ratios = vec![0.2, 0.2];

    let group_id = column.groups[0];
    let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
        panic!("factory should create Tabs");
    };
    tabs.active_index = tabs.children.len();

    let errors = layout.validate(&DockMetrics::default()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidColumn { reason: "ratios must sum to 1.0 within 1e-4", .. }
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidTabs { reason: "active_index is outside children", .. }
    )));
}

#[test]
fn validation_detects_missing_node_and_cycle_even_in_broken_trees() {
    let mut layout = initial_layout(Profile::Generic);
    let root = layout.main_area(DockSide::Right).unwrap().root;
    let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
        panic!("factory should create a column");
    };
    column.groups.push(NodeId::null());
    column.group_ratios.push(0.0);

    let group_id = column.groups[0];
    let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
        panic!("factory should create Tabs");
    };
    tabs.children.push(group_id);

    let orphan_panel = layout
        .nodes
        .insert(Node::Panel(Panel::new(PanelId::null())));

    let errors = layout.validate(&DockMetrics::default()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::MissingNode { missing, .. } if *missing == NodeId::null()
    )));
    assert!(errors.iter().any(|error| matches!(error, ValidationError::NodeCycle { .. })));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::MultipleParents { node, .. } if *node == group_id
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::UnreachableNode { node } if *node == orphan_panel
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::MissingPanelInstance { node, panel }
            if *node == orphan_panel && *panel == PanelId::null()
    )));
}

#[test]
fn validation_detects_duplicate_panel_placement_and_singleton_keys() {
    let mut layout = initial_layout(Profile::Generic);
    let ids: Vec<_> = layout.nodes.iter().filter_map(|(id, node)| match node {
        Node::Panel(panel) => Some((id, panel.panel_id)),
        _ => None,
    }).collect();
    let (first_node, first_panel) = ids[0];
    let (second_node, second_panel) = ids[1];
    *layout.nodes.get_mut(second_node).unwrap() = Node::Panel(Panel::new(first_panel));

    let duplicate_instance = layout.panel_instances.get_mut(second_panel).unwrap();
    duplicate_instance.key = PanelKey::new("layers").unwrap();
    duplicate_instance.constraints.singleton = true;
    layout.panel_instances.get_mut(first_panel).unwrap().constraints.singleton = true;

    let errors = layout.validate(&DockMetrics::default()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::DuplicatePanelPlacement { panel, .. } if *panel == first_panel
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::DuplicateSingletonPanel { .. }
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::UnplacedPanelInstance { panel } if *panel == second_panel
    )));
    assert_ne!(first_node, second_node);
}

#[test]
fn photoshop_profile_rejects_vertical_main_split() {
    let mut layout = initial_layout(Profile::Photoshop);
    let root = layout.main_area(DockSide::Right).unwrap().root;
    let children = match layout.nodes.get(root).unwrap() {
        Node::Column(column) => column.groups.clone(),
        _ => unreachable!(),
    };
    *layout.nodes.get_mut(root).unwrap() = Node::Split(Split {
        axis: Axis::Y,
        children: children.clone(),
        ratios: vec![0.5, 0.5],
    });

    let errors = layout.validate(&DockMetrics::default()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidMainRoot { reason: "main Split must use axis X", .. }
    )));
}

#[test]
fn floating_bounds_and_duplicate_z_order_are_validated() {
    let mut nodes = SlotMap::<NodeId, Node>::with_key();
    let root = nodes.insert(Node::Tabs(Tabs {
        children: Vec::new(),
        active_index: 0,
        collapsed: false,
    }));
    let panels = SlotMap::<PanelId, PanelInstance>::with_key();
    let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
    let area_one = areas.insert(DockArea::floating(root));
    let area_two = areas.insert(DockArea::floating(root));
    let mut floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
    let bounds = Rect::new(Point::ZERO, Size::new(0.0, 300.0));
    floating.insert(FloatingGroup::new(area_one, bounds, 1));
    floating.insert(FloatingGroup::new(area_two, Rect::new(Point::ZERO, Size::new(400.0, 300.0)), 1));

    let layout = Layout::from_parts(
        nodes,
        panels,
        areas,
        [None, None],
        floating,
        CanvasState::default(),
        Profile::Generic,
        0,
        ChromeMode::Visible,
    );
    let errors = layout.validate(&DockMetrics::default()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidFloatingGroup { reason: "bounds must have positive dimensions", .. }
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::DuplicateFloatingZOrder { z_order: 1, .. }
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidTabs { .. }
    )));
}

#[test]
fn validation_rejects_invalid_metrics_and_canvas_width() {
    let mut layout = Layout::new(Profile::Generic);
    layout.canvas = CanvasState::new(Some(Rect::new(Point::ZERO, Size::new(200.0, 600.0))));
    let errors = layout.validate(&DockMetrics::default()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidCanvas { reason: "canvas width is below min_canvas_width" }
    )));

    let metrics = DockMetrics {
        max_columns: 0,
        ..DockMetrics::default()
    };
    let errors = layout.validate(&metrics).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidMetrics { .. }
    )));
}
