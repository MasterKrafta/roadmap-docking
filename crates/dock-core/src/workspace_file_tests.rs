use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, CanvasState, ChromeMode, ColumnMode, DockMetrics, DockSide, Layout, PanelConstraints,
    PanelKey, PanelState, Point, Profile, Rect, Size, WorkspaceDiagnostic, WorkspaceFile,
    WorkspaceFileError, WorkspaceLoadOptions, WorkspaceNode, WORKSPACE_VERSION,
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
    let (_, layers_node) = builder.panel(key("layers"), constraints());
    let (_, color_node) = builder.panel(key("color"), constraints());
    let (_, history_node) = builder.panel(key("history"), constraints());

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

    let descriptors = ["layers", "color", "history"]
        .into_iter()
        .map(|panel_key| (key(panel_key), constraints()))
        .collect();
    (builder.build(), descriptors)
}

fn find_column_mut(node: &mut WorkspaceNode) -> Option<&mut Vec<f32>> {
    match node {
        WorkspaceNode::Column { group_ratios, .. } => Some(group_ratios),
        WorkspaceNode::Split { children, .. } | WorkspaceNode::Tabs { children, .. } => {
            children.iter_mut().find_map(find_column_mut)
        }
        WorkspaceNode::Panel { .. } => None,
    }
}

#[test]
fn versioned_json_round_trip_preserves_structure_without_runtime_keys() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    assert_eq!(file.version, WORKSPACE_VERSION);

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
    assert!(json.contains(&format!("\"version\": {WORKSPACE_VERSION}")));
    assert!(json.contains("\"type\": \"column\""));
    assert!(json.contains("\"key\": \"layers\""));
    assert!(!json.contains("NodeId"));
    assert!(!json.contains("PanelId"));
    assert!(!json.contains("DockAreaId"));
    assert!(!json.contains("FloatingGroupId"));

    let decoded = WorkspaceFile::from_json(&json).unwrap();
    assert_eq!(decoded, file);
    let report = decoded.into_layout(&descriptors, &metrics).unwrap();
    assert!(report.diagnostics.is_empty());
    let restored = report.layout;

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
fn version_one_workspace_migrates_to_current_version_with_diagnostic() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    file.version = 1;
    let old_json = serde_json::to_string(&file).unwrap();

    let (migrated, parse_diagnostics) =
        WorkspaceFile::from_json_with_diagnostics(&old_json).unwrap();
    assert_eq!(migrated.version, WORKSPACE_VERSION);
    assert!(parse_diagnostics.contains(&WorkspaceDiagnostic::SchemaMigrated {
        from: 1,
        to: WORKSPACE_VERSION,
    }));

    let report = WorkspaceFile::load_json(
        &old_json,
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .unwrap();
    assert!(report.diagnostics.contains(&WorkspaceDiagnostic::SchemaMigrated {
        from: 1,
        to: WORKSPACE_VERSION,
    }));
    assert_eq!(report.layout.validate(&metrics), Ok(()));
}

#[test]
fn unsupported_file_versions_and_malformed_json_return_errors() {
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    file.version = 99;
    let json = serde_json::to_string(&file).unwrap();

    assert!(matches!(
        WorkspaceFile::from_json(&json),
        Err(WorkspaceFileError::UnsupportedVersion {
            found: 99,
            supported: WORKSPACE_VERSION,
        })
    ));
    assert!(matches!(
        WorkspaceFile::load_json(
            "{not json",
            &descriptors,
            &DockMetrics::default(),
            WorkspaceLoadOptions::default(),
        ),
        Err(WorkspaceFileError::Json(_))
    ));
}

#[test]
fn unknown_panel_keys_are_preserved_as_placeholders_with_warning() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let file = WorkspaceFile::from_layout(&layout).unwrap();
    let known_descriptors = descriptors
        .into_iter()
        .filter(|(panel_key, _)| panel_key.as_str() != "history")
        .collect::<Vec<_>>();
    let json = file.to_json_pretty().unwrap();

    let report = WorkspaceFile::load_json(
        &json,
        &known_descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .unwrap();

    assert!(report.diagnostics.contains(
        &WorkspaceDiagnostic::UnknownPanelKeyPreserved {
            key: "history".to_owned()
        }
    ));
    assert!(report.layout.validate(&metrics).is_ok());
    assert_eq!(
        report
            .layout
            .panel_instances()
            .map(|(_, panel)| panel.key.as_str())
            .filter(|panel_key| *panel_key == "history")
            .count(),
        1
    );
}

#[test]
fn safe_ratio_repair_is_normalized_and_reported() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    let root = file.layout.right.as_mut().unwrap();
    *find_column_mut(root).expect("main root has a column") = vec![0.25];
    let json = serde_json::to_string(&file).unwrap();

    let report = WorkspaceFile::load_json(
        &json,
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .unwrap();
    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("ratios must sum to 1.0")))
    }));
    assert!(report.layout.validate(&metrics).is_ok());
    let root = report.layout.main_area(DockSide::Right).unwrap().root;
    let Some(crate::Node::Column(column)) = report.layout.node(root) else {
        panic!("main root must remain a column after safe normalization");
    };
    assert_eq!(column.group_ratios, vec![1.0]);
}

#[test]
fn invalid_profile_specific_tree_is_reported_instead_of_guessed_at() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    file.layout.right = Some(WorkspaceNode::Split {
        axis: Axis::Y,
        children: vec![WorkspaceNode::Tabs {
            children: vec![WorkspaceNode::Panel {
                key: key("layers"),
                state: PanelState::default(),
            }],
            active_index: 0,
            collapsed: false,
        }],
        ratios: vec![1.0],
    });

    let result = WorkspaceFile::load_json(
        &serde_json::to_string(&file).unwrap(),
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    );
    assert!(matches!(result, Err(WorkspaceFileError::InvalidLayout(_))));
}

#[test]
fn floating_bounds_are_clamped_to_current_visible_client_region() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let file = WorkspaceFile::from_layout(&layout).unwrap();
    let region = Rect::new(Point::new(100.0, 80.0), Size::new(300.0, 200.0));

    let report = WorkspaceFile::load_json(
        &file.to_json_pretty().unwrap(),
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::new(Some(region)),
    )
    .unwrap();

    let expected = Rect::new(Point::new(100.0, 80.0), Size::new(300.0, 200.0));
    assert!(report.diagnostics.contains(
        &WorkspaceDiagnostic::FloatingBoundsClamped {
            index: 0,
            original: Rect::new(Point::new(44.0, 55.0), Size::new(420.0, 320.0)),
            clamped: expected,
        }
    ));
    let floating = report.layout.floating_groups_in_paint_order();
    assert_eq!(floating[0].1.bounds, expected);
    assert!(report.layout.validate(&metrics).is_ok());
}

#[test]
fn stale_profile_rail_width_is_repaired_and_reported() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) = builder.panel(key("layers"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let rail = builder.column(
        metrics.rail_width + 12.0,
        ColumnMode::IconsOnly,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, rail);
    let layout = builder.build();
    let file = WorkspaceFile::from_layout(&layout).unwrap();
    let json = serde_json::to_string(&file).unwrap();

    let report = WorkspaceFile::load_json(
        &json,
        &[(key("layers"), constraints())],
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .unwrap();

    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("IconsOnly width reset")))
    }));
    let root = report.layout.main_area(DockSide::Right).unwrap().root;
    let Some(crate::Node::Column(column)) = report.layout.node(root) else {
        panic!("restored rail must stay a column");
    };
    assert_eq!(column.mode, ColumnMode::IconsOnly);
    assert_eq!(column.width, metrics.rail_width);
    assert!(report.layout.validate(&metrics).is_ok());
}

#[test]
fn invalid_visible_region_returns_an_error() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let file = WorkspaceFile::from_layout(&layout).unwrap();

    assert!(matches!(
        file.into_layout_with_options(
            &descriptors,
            &metrics,
            WorkspaceLoadOptions::new(Some(Rect::ZERO)),
        ),
        Err(WorkspaceFileError::InvalidVisibleClientRegion(_))
    ));
}
