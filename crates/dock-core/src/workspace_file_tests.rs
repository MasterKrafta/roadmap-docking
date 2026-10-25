use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, CanvasState, ChromeMode, ColumnMode, DockMetrics, DockSide, GeometryError, Layout,
    PanelConstraints, PanelKey, PanelState, Point, Profile, Rect, Size, WORKSPACE_VERSION,
    WorkspaceDiagnostic, WorkspaceFile, WorkspaceFileError, WorkspaceLoadOptions, WorkspaceNode,
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
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
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
        Some(Rect::new(Point::new(280.0, 0.0), Size::new(800.0, 600.0)))
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
    assert_eq!(panel.state.last_known_size(), Some(Size::new(240.0, 190.0)));

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
    let Some(crate::Node::Panel(floating_panel_leaf)) = restored.node(floating_tabs.children[0])
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
    let file = WorkspaceFile::from_layout(&layout).unwrap();
    let mut legacy: serde_json::Value = serde_json::to_value(&file).unwrap();
    legacy["version"] = serde_json::json!(1);
    legacy["layout"]["right"]
        .as_object_mut()
        .unwrap()
        .remove("last_expanded_width");
    let old_json = legacy.to_string();

    let (migrated, parse_diagnostics) =
        WorkspaceFile::from_json_with_diagnostics(&old_json).unwrap();
    assert_eq!(migrated.version, WORKSPACE_VERSION);
    assert!(
        parse_diagnostics.contains(&WorkspaceDiagnostic::SchemaMigrated {
            from: 1,
            to: WORKSPACE_VERSION,
        })
    );
    assert!(matches!(
        migrated.layout.right,
        Some(WorkspaceNode::Column {
            last_expanded_width: Some(width),
            ..
        }) if width == 280.0
    ));

    let report = WorkspaceFile::load_json(
        &old_json,
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .unwrap();
    assert!(
        report
            .diagnostics
            .contains(&WorkspaceDiagnostic::SchemaMigrated {
                from: 1,
                to: WORKSPACE_VERSION,
            })
    );
    assert_eq!(report.layout.validate(&metrics), Ok(()));
}

#[test]
fn migrated_recoverable_damage_is_normalized_and_round_trips_without_losing_unknown_panels() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let known_descriptors = descriptors
        .into_iter()
        .filter(|(panel_key, _)| panel_key.as_str() != "history")
        .collect::<Vec<_>>();

    let file = WorkspaceFile::from_layout(&layout).unwrap();
    let mut legacy: serde_json::Value = serde_json::to_value(&file).unwrap();
    legacy["version"] = serde_json::json!(1);
    legacy["layout"]["right"]
        .as_object_mut()
        .unwrap()
        .remove("last_expanded_width");
    legacy["layout"]["right"]["group_ratios"] = serde_json::json!([0.25]);
    legacy["layout"]["right"]["groups"][0]["active_index"] = serde_json::json!(99);
    let legacy_json = legacy.to_string();

    let report = WorkspaceFile::load_json(
        &legacy_json,
        &known_descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .expect("safe, deterministic v1 data damage should be normalized");

    assert!(report.diagnostics.contains(&WorkspaceDiagnostic::SchemaMigrated {
        from: 1,
        to: WORKSPACE_VERSION,
    }));
    assert!(report.diagnostics.contains(
        &WorkspaceDiagnostic::UnknownPanelKeyPreserved {
            key: "history".to_owned()
        }
    ));
    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("ratios must sum to 1.0"))
                && issues.iter().any(|issue| issue.contains("active_index is outside children")))
    }));

    assert_eq!(report.layout.validate(&metrics), Ok(()));
    report
        .layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(2400.0, 1600.0)),
            &metrics,
        )
        .expect("normalized layout should resolve geometry in a sufficient viewport");

    let right = report.layout.main_area(DockSide::Right).unwrap().root;
    let Some(crate::Node::Column(column)) = report.layout.node(right) else {
        panic!("right root must remain a column after recovery");
    };
    assert_eq!(column.group_ratios, vec![1.0]);
    assert_eq!(column.last_expanded_width, Some(280.0));
    let Some(crate::Node::Tabs(tabs)) = report.layout.node(column.groups[0]) else {
        panic!("first column group must remain tabs after recovery");
    };
    assert_eq!(tabs.active_index, tabs.children.len() - 1);

    assert_eq!(
        report
            .layout
            .panel_instances()
            .filter(|(_, panel)| panel.key.as_str() == "history")
            .count(),
        1,
        "unknown panel key should remain present rather than being discarded"
    );

    // Export the recovered in-memory model as canonical v2 JSON and reopen it.
    let canonical_file = report.layout.to_workspace_file().unwrap();
    assert_eq!(canonical_file.version, WORKSPACE_VERSION);
    let canonical_json = canonical_file.to_json_pretty().unwrap();
    assert_eq!(
        WorkspaceFile::from_json(&canonical_json).unwrap(),
        canonical_file
    );

    let reopened = WorkspaceFile::load_json(
        &canonical_json,
        &known_descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .expect("canonical v2 workspace should reopen after recovery");
    assert_eq!(reopened.layout.validate(&metrics), Ok(()));
    reopened
        .layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(2400.0, 1600.0)),
            &metrics,
        )
        .expect("round-tripped workspace should still resolve geometry");
    assert!(reopened.diagnostics.contains(
        &WorkspaceDiagnostic::UnknownPanelKeyPreserved {
            key: "history".to_owned()
        }
    ));
    assert!(
        !reopened
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, WorkspaceDiagnostic::SchemaMigrated { .. })),
        "canonical v2 workspace should not be migrated again"
    );
    assert!(
        !reopened
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { .. })),
        "canonical v2 workspace should need no further normalization"
    );
    assert_eq!(
        reopened.layout.to_workspace_file().unwrap(),
        canonical_file,
        "recover → serialize → reload should stabilize the workspace representation"
    );
}

#[test]
fn damaged_workspace_schema_is_rejected_before_layout_normalization() {
    let (layout, descriptors) = sample_layout();
    let file = WorkspaceFile::from_layout(&layout).unwrap();
    let original: serde_json::Value = serde_json::to_value(&file).unwrap();

    let mut unknown_node_type = original.clone();
    unknown_node_type["layout"]["right"]["type"] = serde_json::json!("not_a_workspace_node");
    let unknown_node_type = unknown_node_type.to_string();
    assert!(matches!(
        WorkspaceFile::load_json(
            &unknown_node_type,
            &descriptors,
            &DockMetrics::default(),
            WorkspaceLoadOptions::default(),
        ),
        Err(WorkspaceFileError::Json(_))
    ));

    let mut invalid_field_type = original;
    invalid_field_type["layout"]["right"]["width"] = serde_json::json!("wide");
    let invalid_field_type = invalid_field_type.to_string();
    assert!(matches!(
        WorkspaceFile::load_json(
            &invalid_field_type,
            &descriptors,
            &DockMetrics::default(),
            WorkspaceLoadOptions::default(),
        ),
        Err(WorkspaceFileError::Json(_))
    ));
}

#[test]
fn current_schema_missing_expanded_width_is_repaired_with_diagnostic() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let file = WorkspaceFile::from_layout(&layout).unwrap();
    let mut value: serde_json::Value = serde_json::to_value(&file).unwrap();
    value["layout"]["right"]
        .as_object_mut()
        .unwrap()
        .remove("last_expanded_width");
    let json = value.to_string();

    let report = WorkspaceFile::load_json(
        &json,
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .unwrap();

    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("last_expanded_width restored")))
    }));
    let root = report.layout.main_area(DockSide::Right).unwrap().root;
    let Some(crate::Node::Column(column)) = report.layout.node(root) else {
        panic!("main root must be a column");
    };
    assert_eq!(column.last_expanded_width, Some(280.0));
    assert!(report.layout.validate(&metrics).is_ok());
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

    assert!(
        report
            .diagnostics
            .contains(&WorkspaceDiagnostic::UnknownPanelKeyPreserved {
                key: "history".to_owned()
            })
    );
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
fn out_of_range_active_tab_is_clamped_to_last_child_and_reported() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    let Some(WorkspaceNode::Column { groups, .. }) = file.layout.right.as_mut() else {
        panic!("main root must serialize as a column");
    };
    let Some(WorkspaceNode::Tabs { active_index, .. }) = groups.first_mut() else {
        panic!("column group must serialize as tabs");
    };
    *active_index = 99;
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
            if issues.iter().any(|issue| issue.contains("active_index is outside children")))
    }));
    let root = report.layout.main_area(DockSide::Right).unwrap().root;
    let Some(crate::Node::Column(column)) = report.layout.node(root) else {
        panic!("main root must remain a column");
    };
    let Some(crate::Node::Tabs(tabs)) = report.layout.node(column.groups[0]) else {
        panic!("first group must remain tabs");
    };
    assert_eq!(tabs.active_index, tabs.children.len() - 1);
    assert!(report.layout.validate(&metrics).is_ok());
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
    assert!(
        report
            .diagnostics
            .contains(&WorkspaceDiagnostic::FloatingBoundsClamped {
                index: 0,
                original: Rect::new(Point::new(44.0, 55.0), Size::new(420.0, 320.0)),
                clamped: expected,
            })
    );
    let floating = report.layout.floating_groups_in_paint_order();
    assert_eq!(floating[0].1.bounds, expected);
    assert!(report.layout.validate(&metrics).is_ok());
}

#[test]
fn client_region_smaller_than_floating_minimum_is_reported_not_shrunk_below_minimum() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let file = WorkspaceFile::from_layout(&layout);

    assert!(matches!(
        file.unwrap().into_layout_with_options(
            &descriptors,
            &metrics,
            WorkspaceLoadOptions::new(Some(Rect::new(
                Point::ZERO,
                Size::new(1.0, 1.0),
            ))),
        ),
        Err(WorkspaceFileError::FloatingBoundsInsufficientSpace {
            axis: crate::Axis::X,
            available,
            required,
            ..
        }) if available == 1.0 && required >= constraints().min_size.width
    ));
}

#[test]
fn stale_profile_rail_width_is_repaired_and_reported() {
    let metrics = DockMetrics::default();
    let descriptors = vec![(key("layers"), constraints())];

    for profile in [Profile::Generic, Profile::Photoshop] {
        let mut builder = LayoutBuilder::new(profile);
        let (_, panel_node) = builder.panel(key("layers"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let rail = builder.column(
            metrics.rail_width + 12.0,
            ColumnMode::IconsOnly,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, rail);
        let file = WorkspaceFile::from_layout(&builder.build()).unwrap();
        let json = serde_json::to_string(&file).unwrap();

        let report = WorkspaceFile::load_json(
            &json,
            &descriptors,
            &metrics,
            WorkspaceLoadOptions::default(),
        )
        .unwrap();

        assert_eq!(report.layout.profile(), profile);
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
}

#[test]
fn invalid_visible_region_returns_an_error() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();

    for region in [
        Rect::ZERO,
        // Positive finite dimensions can disappear when added at a large f32
        // origin. Workspace clamping must reject these regions without panic.
        Rect::new(Point::new(16_777_216.0, 0.0), Size::new(1.0, 100.0)),
        Rect::new(Point::new(0.0, 16_777_216.0), Size::new(100.0, 1.0)),
        Rect::new(Point::new(f32::MAX, 0.0), Size::new(1.0, 100.0)),
    ] {
        let fresh_file = WorkspaceFile::from_layout(&layout).unwrap();
        assert!(matches!(
            fresh_file.into_layout_with_options(
                &descriptors,
                &metrics,
                WorkspaceLoadOptions::new(Some(region)),
            ),
            Err(WorkspaceFileError::InvalidVisibleClientRegion(
                GeometryError::NonPositiveDimension
            ))
        ));
    }
}

#[test]
fn workspace_load_expands_narrow_expanded_column_and_preserves_restore_width() {
    let metrics = DockMetrics::default();
    let wide_constraints = PanelConstraints::new(
        Size::new(400.0, 80.0),
        Size::new(450.0, 140.0),
        Some(Size::new(600.0, 300.0)),
        false,
    );

    for profile in [Profile::Generic, Profile::Photoshop] {
        let mut builder = LayoutBuilder::new(profile);
        let (_, panel_node) = builder.panel(key("wide-panel"), wide_constraints);
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Right, column);
        let source = builder.build();
        assert!(source.validate(&metrics).is_err());
        assert!(matches!(
            source.compute_rects(
                Rect::new(Point::new(0.0, 0.0), Size::new(1000.0, 600.0)),
                &metrics,
            ),
            Err(crate::ComputeRectsError::InvalidLayout)
        ));

        let mut file = WorkspaceFile::from_layout(&source).unwrap();
        let Some(WorkspaceNode::Column {
            width,
            last_expanded_width,
            ..
        }) = file.layout.right.as_mut()
        else {
            panic!("workspace main root must be a Column");
        };
        *width = 280.0;
        *last_expanded_width = Some(500.0);

        let descriptors = vec![(key("wide-panel"), wide_constraints)];
        // Exercise the actual serde JSON boundary instead of calling into_layout
        // directly, so width restoration and profile preservation are covered
        // by the same path users take when reopening a workspace file.
        let json = serde_json::to_string(&file).unwrap();
        let report = WorkspaceFile::load_json(
            &json,
            &descriptors,
            &metrics,
            WorkspaceLoadOptions::default(),
        )
        .unwrap();
        assert_eq!(report.layout.profile(), profile);

        let root = report.layout.main_area(DockSide::Right).unwrap().root;
        let Some(crate::Node::Column(restored)) = report.layout.node(root) else {
            panic!("restored main root must be a Column");
        };

        assert_eq!(restored.width, 400.0);
        assert_eq!(restored.last_expanded_width, Some(500.0));
        assert!(report.diagnostics.iter().any(|diagnostic| {
            matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
                if issues.iter().any(|issue| issue.contains("width raised to content minimum")))
        }));
        assert_eq!(report.layout.validate(&metrics), Ok(()));
        report
            .layout
            .compute_rects(
                Rect::new(Point::new(0.0, 0.0), Size::new(1000.0, 600.0)),
                &metrics,
            )
            .expect("restored layout must be placeable after the width repair");
    }
}

#[test]
fn workspace_load_reports_conflicting_expanded_content_minimum_and_maximum() {
    let metrics = DockMetrics::default();
    let wide_constraints = PanelConstraints::new(
        Size::new(400.0, 80.0),
        Size::new(420.0, 220.0),
        Some(Size::new(450.0, 300.0)),
        false,
    );
    let narrow_constraints = PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(200.0, 220.0),
        Some(Size::new(350.0, 300.0)),
        false,
    );
    let descriptors = vec![
        (key("wide-panel"), wide_constraints),
        (key("narrow-panel"), narrow_constraints),
    ];

    for profile in [Profile::Generic, Profile::Photoshop] {
        let mut builder = LayoutBuilder::new(profile);
        let (_, wide_panel) = builder.panel(key("wide-panel"), wide_constraints);
        let (_, narrow_panel) = builder.panel(key("narrow-panel"), narrow_constraints);
        let group = builder.tabs(vec![wide_panel, narrow_panel], 0, false);
        let column = builder.column(400.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Right, column);
        let file = WorkspaceFile::from_layout(&builder.build()).unwrap();
        let json = serde_json::to_string(&file).unwrap();

        assert!(matches!(
            WorkspaceFile::load_json(
                &json,
                &descriptors,
                &metrics,
                WorkspaceLoadOptions::default(),
            ),
            Err(WorkspaceFileError::InvalidLayout(errors))
                if errors.iter().any(|error| error.to_string().contains(
                    "minimum width required by expanded contents exceeds panel maximum width"
                ))
        ), "incompatible min/max constraints must fail for {profile:?}");
    }
}

#[test]
fn workspace_load_clears_stale_canvas_bounds_when_metrics_require_more_width() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    file.layout.canvas = CanvasState::new(Some(Rect::new(
        Point::new(12.0, 0.0),
        Size::new(metrics.min_canvas_width - 40.0, 600.0),
    )));

    let report = WorkspaceFile::load_json(
        &file.to_json_pretty().unwrap(),
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .expect("a stale canvas measurement should not reject the workspace");

    assert_eq!(report.layout.canvas().last_known_bounds(), None);
    assert!(report.layout.validate(&metrics).is_ok());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("canvas last_known_bounds cleared")))
    }));
}

#[test]
fn workspace_load_keeps_first_singleton_panel_and_removes_duplicate_with_diagnostic() {
    let metrics = DockMetrics::default();
    let (layout, mut descriptors) = sample_layout();
    for (panel_key, constraints) in &mut descriptors {
        if panel_key.as_str() == "layers" {
            constraints.singleton = true;
        }
    }

    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    let Some(WorkspaceNode::Column { groups, .. }) = file.layout.right.as_mut() else {
        panic!("right main area must serialize as a column");
    };
    let Some(WorkspaceNode::Tabs { children, .. }) = groups.first_mut() else {
        panic!("column group must serialize as tabs");
    };
    let duplicate = children.first().expect("first tab exists").clone();
    children.push(duplicate);

    let report = WorkspaceFile::load_json(
        &file.to_json_pretty().unwrap(),
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .expect("a duplicate singleton should be repaired without losing the first panel");

    assert_eq!(
        report
            .layout
            .panel_instances()
            .filter(|(_, instance)| instance.key.as_str() == "layers")
            .count(),
        1
    );
    let right = report.layout.main_area(DockSide::Right).unwrap().root;
    let Some(crate::Node::Column(column)) = report.layout.node(right) else {
        panic!("right main root must remain a column");
    };
    let Some(crate::Node::Tabs(tabs)) = report.layout.node(column.groups[0]) else {
        panic!("first column group must remain tabs");
    };
    assert_eq!(tabs.children.len(), 2);
    assert!(report.layout.validate(&metrics).is_ok());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("duplicate instance") && issue.contains("kept")))
    }));
}

#[test]
fn workspace_load_merges_excess_main_columns_without_losing_panels() {
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    let first_column = file.layout.right.take().unwrap();
    file.layout.right = Some(WorkspaceNode::Split {
        axis: Axis::X,
        children: vec![first_column.clone(), first_column.clone(), first_column],
        ratios: vec![1.0 / 3.0; 3],
    });
    let metrics = DockMetrics {
        max_columns: 2,
        ..DockMetrics::default()
    };

    let report = WorkspaceFile::load_json(
        &file.to_json_pretty().unwrap(),
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .expect("a workspace with more columns than the current limit should be repaired");

    let right = report.layout.main_area(DockSide::Right).unwrap().root;
    let Some(crate::Node::Split(split)) = report.layout.node(right) else {
        panic!("two retained main columns should remain in a horizontal split");
    };
    assert_eq!(split.children.len(), 2);
    assert_eq!(report.layout.panel_instances().count(), 7);
    assert!(report.layout.validate(&metrics).is_ok());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("merged into column") && issue.contains("max_columns")))
    }));
}

#[test]
fn workspace_load_renumbers_duplicate_floating_z_orders_and_preserves_paint_order() {
    let metrics = DockMetrics::default();
    let (layout, descriptors) = sample_layout();
    let mut file = WorkspaceFile::from_layout(&layout).unwrap();
    let duplicate = file
        .layout
        .floating_groups
        .first()
        .expect("sample has one floating group")
        .clone();
    file.layout.floating_groups.push(duplicate);

    let report = WorkspaceFile::load_json(
        &file.to_json_pretty().unwrap(),
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .expect("duplicate floating z-order values should be repaired");

    let z_orders: Vec<_> = report
        .layout
        .floating_groups_in_paint_order()
        .iter()
        .map(|(_, group)| group.z_order)
        .collect();
    assert_eq!(z_orders, vec![0, 1]);
    assert!(report.layout.validate(&metrics).is_ok());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, WorkspaceDiagnostic::LayoutNormalized { issues }
            if issues.iter().any(|issue| issue.contains("duplicate stacking order")))
    }));
}

