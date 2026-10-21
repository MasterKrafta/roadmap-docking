use crate::layout_builder::LayoutBuilder;
use crate::{
    CanvasState, ChromeMode, ColumnMode, DockMetrics, DockSide, Layout, PanelConstraints, PanelKey,
    PanelState, Point, Profile, Rect, Size, WorkspaceFile, WorkspaceFileError, WorkspaceNode,
};

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).unwrap()
}

fn constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(80.0, 60.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        false,
    )
}

fn sample_layout() -> (Layout, Vec<(PanelKey, PanelConstraints)>) {
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (layers_id, layers_node) = builder.panel(key("layers"), constraints());
    let (color_id, color_node) = builder.panel(key("color"), constraints());
    let (history_id, history_node) = builder.panel(key("history"), constraints());

    let group = builder.tabs(vec![layers_node, color_node], 1, true);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);

    let floating_root = builder.tabs(vec![history_node], 0, false);
    builder.floating_group(
        floating_root,
        Rect::new(Point::new(44.0, 55.0), Size::new(420.0, 320.0)),
        7,
    );

    let _ = (layers_id, color_id, history_id);
    let descriptors = ["layers", "color", "history"]
        .into_iter()
        .map(|panel_key| (key(panel_key), constraints()))
        .collect();
    (builder.build(), descriptors)
}

#[test]
fn versioned_json_round_trip_preserves_structure_without_runtime_keys() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();

    file.layout.chrome = ChromeMode::DockHidden;
    file.layout.canvas = CanvasState::new(Some(Rect::new(
        Point::new(280.0, 0.0),
        Size::new(800.0, 600.0),
    )));

    let Some(WorkspaceNode::Column { groups, .. }) = file.layout.right.as_mut() else {
        panic!("right main area must serialize as a column");
    };
    let Some(WorkspaceNode::Tabs { children, .. }) = groups.first_mut() else {
        panic!("column group must serialize as tabs");
    };
    let Some(WorkspaceNode::Panel { state, .. }) = children.first_mut() else {
        panic!("first tab must serialize as a panel");
    };
    *state = PanelState::new(Some(Size::new(240.0, 190.0)));

    let json = file.to_json_pretty().unwrap();
    assert!(json.contains("\"format\": \"roadmap-docking-workspace\""));
    assert!(json.contains("\"version\": 1"));
    assert!(json.contains("\"type\": \"column\""));
    assert!(json.contains("\"key\": \"layers\""));
    assert!(!json.contains("NodeId"));
    assert!(!json.contains("PanelId"));
    assert!(!json.contains("DockAreaId"));
    assert!(!json.contains("FloatingGroupId"));

    let decoded = WorkspaceFile::from_json(&json).unwrap();
    assert_eq!(decoded, file);
    let restored = decoded.into_layout(&descriptors, &metrics).unwrap();

    assert_eq!(restored.profile(), Profile::Photoshop);
    assert_eq!(restored.chrome(), ChromeMode::DockHidden);
    assert_eq!(
        restored.canvas().last_known_bounds(),
        Some(Rect::new(
            Point::new(280.0, 0.0),
            Size::new(800.0, 600.0)
        ))
    );
    assert_eq!(restored.version(), 0);
    assert!(restored.flyout().is_none());
    assert_eq!(restored.validate(&metrics), Ok(()));

    let right = restored.main_area(DockSide::Right).unwrap();
    let Some(crate::Node::Column(column)) = restored.node(right.root) else {
        panic!("restored main root must be a column");
    };
    assert_eq!(column.width, 280.0);
    assert_eq!(column.mode, ColumnMode::Expanded);
    assert_eq!(column.group_ratios, vec![1.0]);

    let Some(crate::Node::Tabs(tabs)) = restored.node(column.groups[0]) else {
        panic!("restored group must be tabs");
    };
    assert_eq!(tabs.active_index, 1);
    assert!(tabs.collapsed);
    let Some(crate::Node::Panel(panel_leaf)) = restored.node(tabs.children[0]) else {
        panic!("restored tab child must be a panel");
    };
    let panel = restored.panel_instance(panel_leaf.panel_id).unwrap();
    assert_eq!(panel.key.as_str(), "layers");
    assert_eq!(
        panel.state.last_known_size(),
        Some(Size::new(240.0, 190.0))
    );

    let floating = restored.floating_groups_in_paint_order();
    assert_eq!(floating.len(), 1);
    assert_eq!(floating[0].1.z_order, 7);
    assert_eq!(
        floating[0].1.bounds,
        Rect::new(Point::new(44.0, 55.0), Size::new(420.0, 320.0))
    );
    let floating_area = restored.dock_area(floating[0].1.area).unwrap();
    let Some(crate::Node::Tabs(floating_tabs)) = restored.node(floating_area.root) else {
        panic!("restored floating root must be tabs");
    };
    let Some(crate::Node::Panel(floating_panel_leaf)) =
        restored.node(floating_tabs.children[0])
    else {
        panic!("restored floating tab child must be a panel");
    };
    assert_eq!(
        restored
            .panel_instance(floating_panel_leaf.panel_id)
            .unwrap()
            .key
            .as_str(),
        "history"
    );
}

#[test]
fn unsupported_file_versions_are_rejected_before_layout_rebuild() {
    let (layout, _) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    file.version = 99;
    let json = serde_json::to_string(&file).unwrap();

    assert!(matches!(
        WorkspaceFile::from_json(&json),
        Err(WorkspaceFileError::UnsupportedVersion {
            found: 99,
            supported: 1
        })
    ));
}

#[test]
fn missing_panel_descriptor_is_reported_without_inventing_constraints() {
    let metrics = DockMetrics::default();
    let (layout, _) = sample_layout();
    let file = WorkspaceFile::from_layout(&layout).unwrap();

    assert!(matches!(
        file.into_layout(&[], &metrics),
        Err(WorkspaceFileError::MissingPanelDescriptor { key }) if key == "layers"
    ));
}
