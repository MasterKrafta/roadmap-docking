use slotmap::Key;

use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ChromeMode, ColumnMode, DockCommand, DockMetrics, DockSide, DropTarget, Layout, Node,
    PanelConstraints, PanelId, PanelKey, Point, Profile, Rect, Size, UndoRedoHistory,
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

struct Fixture {
    layout: Layout,
    column: crate::NodeId,
    group_a: crate::NodeId,
    group_b: crate::NodeId,
    layers: PanelId,
    color: PanelId,
    properties: PanelId,
    floating_a: crate::FloatingGroupId,
}

fn fixture() -> Fixture {
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (layers, layers_node) = builder.panel(key("layers"), constraints());
    let (color, color_node) = builder.panel(key("color"), constraints());
    let (properties, properties_node) = builder.panel(key("properties"), constraints());
    let (_, history_node) = builder.panel(key("history"), constraints());
    let (_, assets_node) = builder.panel(key("assets"), constraints());

    let group_a = builder.tabs(vec![layers_node, color_node], 0, false);
    let group_b = builder.tabs(vec![properties_node, history_node], 0, false);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![group_a, group_b],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);

    let floating_a_root = builder.tabs(vec![assets_node], 0, false);
    let floating_a = builder.floating_group(
        floating_a_root,
        Rect::new(Point::new(30.0, 40.0), Size::new(400.0, 300.0)),
        1,
    );
    let (_, second_float_panel) = builder.panel(key("brushes"), constraints());
    let floating_b_root = builder.tabs(vec![second_float_panel], 0, false);
    builder.floating_group(
        floating_b_root,
        Rect::new(Point::new(70.0, 90.0), Size::new(360.0, 260.0)),
        2,
    );

    Fixture {
        layout: builder.build(),
        column,
        group_a,
        group_b,
        layers,
        color,
        properties,
        floating_a,
    }
}

fn history_benchmark_layout(
    main_columns: usize,
    groups_per_column: usize,
    panels_per_group: usize,
    floating_windows: usize,
    panels_per_floating: usize,
) -> (Layout, crate::NodeId) {
    assert!(main_columns > 0);
    assert!(groups_per_column > 0);
    assert!(panels_per_group > 0);
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let mut columns = Vec::with_capacity(main_columns);
    let mut first_column = None;

    for column_index in 0..main_columns {
        let mut groups = Vec::with_capacity(groups_per_column);
        for group_index in 0..groups_per_column {
            let mut panel_nodes = Vec::with_capacity(panels_per_group);
            for panel_index in 0..panels_per_group {
                let panel_key = format!(
                    "history-bench-main-{column_index}-{group_index}-{panel_index}"
                );
                let (_, panel_node) = builder.panel(key(&panel_key), constraints());
                panel_nodes.push(panel_node);
            }
            groups.push(builder.tabs(panel_nodes, 0, false));
        }

        let column = builder.column(
            280.0,
            ColumnMode::Expanded,
            groups,
            vec![1.0 / groups_per_column as f32; groups_per_column],
        );
        if first_column.is_none() {
            first_column = Some(column);
        }
        columns.push(column);
    }

    let main_root = if columns.len() == 1 {
        columns[0]
    } else {
        builder.split(
            Axis::X,
            columns,
            vec![1.0 / main_columns as f32; main_columns],
        )
    };
    builder.main_area(DockSide::Right, main_root);

    for floating_index in 0..floating_windows {
        let mut panel_nodes = Vec::with_capacity(panels_per_floating);
        for panel_index in 0..panels_per_floating {
            let panel_key = format!("history-bench-floating-{floating_index}-{panel_index}");
            let (_, panel_node) = builder.panel(key(&panel_key), constraints());
            panel_nodes.push(panel_node);
        }
        let tabs = builder.tabs(panel_nodes, 0, false);
        builder.floating_group(
            tabs,
            Rect::new(
                Point::new(40.0 + floating_index as f32 * 12.0, 50.0 + floating_index as f32 * 9.0),
                Size::new(400.0, 300.0),
            ),
            floating_index as i32,
        );
    }

    (
        builder.build(),
        first_column.expect("benchmark layout must contain a main column"),
    )
}

fn report_history_benchmark(
    label: &str,
    nodes: usize,
    panels: usize,
    dock_areas: usize,
    floating_windows: usize,
    iterations: usize,
    mut operation: impl FnMut(),
) {
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        operation();
    }
    let elapsed = start.elapsed();
    eprintln!(
        "history benchmark [{label}]: nodes={nodes}, panel_instances={panels}, dock_areas={dock_areas}, floating_windows={floating_windows}, iterations={iterations}, elapsed_ms={:.3}, us/call={:.3}",
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64,
    );
}

fn node_shape(layout: &Layout, id: crate::NodeId) -> String {
    match layout.node(id) {
        Some(Node::Split(split)) => {
            let children = split
                .children
                .iter()
                .map(|child| node_shape(layout, *child))
                .collect::<Vec<_>>();
            format!("Split({:?},{children:?},{:?})", split.axis, split.ratios)
        }
        Some(Node::Tabs(tabs)) => {
            let children = tabs
                .children
                .iter()
                .map(|child| node_shape(layout, *child))
                .collect::<Vec<_>>();
            format!(
                "Tabs(active={},collapsed={},children={children:?})",
                tabs.active_index, tabs.collapsed
            )
        }
        Some(Node::Panel(panel_leaf)) => {
            let Some(panel) = layout.panel_instance(panel_leaf.panel_id) else {
                return "Panel(<missing-instance>)".to_owned();
            };
            format!(
                "Panel(key={},constraints={:?},state={:?})",
                panel.key, panel.constraints, panel.state
            )
        }
        Some(Node::Column(column)) => {
            let groups = column
                .groups
                .iter()
                .map(|group| node_shape(layout, *group))
                .collect::<Vec<_>>();
            format!(
                "Column(width={},mode={:?},ratios={:?},last_expanded={:?},groups={groups:?})",
                column.width, column.mode, column.group_ratios, column.last_expanded_width
            )
        }
        None => "<missing-node>".to_owned(),
    }
}

fn persistent_shape(layout: &Layout) -> String {
    let main = [DockSide::Left, DockSide::Right].map(|side| {
        layout
            .main_area(side)
            .map(|area| node_shape(layout, area.root))
    });
    let floating: Vec<_> = layout
        .floating_groups_in_paint_order()
        .into_iter()
        .map(|(_, group)| {
            let root = layout
                .dock_area(group.area)
                .map(|area| node_shape(layout, area.root));
            (group.bounds, group.z_order, root)
        })
        .collect();
    let mut remembered_locations: Vec<_> = layout
        .panel_instances()
        .map(|(_, panel)| {
            (
                panel.key.as_str().to_owned(),
                layout.last_dock_location(&panel.key),
            )
        })
        .collect();
    remembered_locations.sort_by(|left, right| left.0.cmp(&right.0));

    format!(
        "profile={:?};chrome={:?};canvas={:?};main={main:?};floating={floating:?};locations={remembered_locations:?}",
        layout.profile(),
        layout.chrome(),
        layout.canvas(),
    )
}

fn assert_round_trip(
    layout: &mut Layout,
    history: &mut UndoRedoHistory,
    command: DockCommand,
    metrics: &DockMetrics,
) {
    let before = persistent_shape(layout);
    let before_version = layout.version();
    let outcome = history.apply(layout, command, metrics).unwrap();
    assert_ne!(
        outcome.new_version, before_version,
        "round-trip test command must make a persistent change"
    );
    let after = persistent_shape(layout);
    assert_ne!(after, before, "command outcome must alter persistent state");
    let after_version = layout.version();

    assert!(history.undo(layout, metrics).unwrap());
    assert_eq!(persistent_shape(layout), before);
    assert!(
        layout.version() > after_version,
        "undo must advance revision"
    );

    assert!(history.redo(layout, metrics).unwrap());
    assert_eq!(persistent_shape(layout), after);
    assert!(
        layout.version() > after_version,
        "redo must advance revision"
    );

    assert!(history.undo(layout, metrics).unwrap());
    assert_eq!(persistent_shape(layout), before);
}

#[test]
fn command_sequence_is_atomic_and_uses_one_undo_checkpoint() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::new();
    let before = persistent_shape(&fixture.layout);

    let outcomes = history
        .apply_sequence(
            &mut fixture.layout,
            [
                DockCommand::ResizeColumn {
                    column: fixture.column,
                    width: 219.0,
                },
                DockCommand::SetColumnMode {
                    column: fixture.column,
                    mode: ColumnMode::IconsOnly,
                },
            ],
            &metrics,
        )
        .unwrap();

    assert_eq!(outcomes.len(), 2);
    assert_eq!(history.undo_len(), 1);
    assert!(matches!(
        fixture.layout.node(fixture.column),
        Some(Node::Column(column))
            if column.mode == ColumnMode::IconsOnly
                && (column.last_expanded_width.unwrap() - 219.0).abs() < 0.01
    ));

    let after = persistent_shape(&fixture.layout);
    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert_eq!(persistent_shape(&fixture.layout), before);
    assert!(!history.can_undo());

    assert!(history.redo(&mut fixture.layout, &metrics).unwrap());
    assert_eq!(persistent_shape(&fixture.layout), after);
    assert_eq!(history.undo_len(), 1);
}

#[test]
fn failed_command_sequence_leaves_layout_and_history_unchanged() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::new();
    let before = persistent_shape(&fixture.layout);
    let before_version = fixture.layout.version();

    let result = history.apply_sequence(
        &mut fixture.layout,
        [
            DockCommand::ResizeColumn {
                column: fixture.column,
                width: 320.0,
            },
            DockCommand::CloseGroup {
                group: fixture.column,
            },
        ],
        &metrics,
    );

    assert!(result.is_err());
    assert_eq!(persistent_shape(&fixture.layout), before);
    assert_eq!(fixture.layout.version(), before_version);
    assert_eq!(history.undo_len(), 0);
    assert_eq!(history.redo_len(), 0);
}

/// Manual release-mode probe for Layout copies, resize candidates, and bounded-history costs.
///
/// Run with:
/// cargo test -p dock-core --release benchmark_history_snapshot_and_bounded_history -- --ignored --nocapture --test-threads=1
///
/// Layout creation and history pre-filling are excluded from timed sections. Reported layout
/// counts describe structure; they are not estimates of allocated bytes.
#[test]
#[ignore = "manual performance probe; run the documented release-mode command"]
fn benchmark_history_snapshot_and_bounded_history() {
    let metrics = DockMetrics::default();
    let scenarios = [
        ("small", 1, 2, 2, 1, 2, 10_000usize),
        ("medium", 3, 4, 4, 2, 4, 5_000usize),
        ("large", 4, 6, 8, 4, 8, 2_000usize),
    ];

    for (name, columns, groups, panels_per_group, floats, panels_per_float, iterations) in scenarios
    {
        let (layout, resize_column) = history_benchmark_layout(
            columns,
            groups,
            panels_per_group,
            floats,
            panels_per_float,
        );
        layout
            .validate(&metrics)
            .expect("benchmark layout must satisfy normal layout invariants");
        let nodes = layout.nodes().count();
        let panels = layout.panel_instances().count();
        let dock_areas = layout.dock_areas().count();
        let floating_windows = layout.floating_groups().count();

        report_history_benchmark(
            &format!("{name}/Layout::clone"),
            nodes,
            panels,
            dock_areas,
            floating_windows,
            iterations,
            || {
                std::hint::black_box(layout.clone());
            },
        );
        report_history_benchmark(
            &format!("{name}/Layout::history_snapshot"),
            nodes,
            panels,
            dock_areas,
            floating_windows,
            iterations,
            || {
                std::hint::black_box(layout.history_snapshot());
            },
        );

        let mut alternate_preview_width = false;
        report_history_benchmark(
            &format!("{name}/clone+ResizeColumn candidate"),
            nodes,
            panels,
            dock_areas,
            floating_windows,
            iterations,
            || {
                alternate_preview_width = !alternate_preview_width;
                let mut candidate = layout.clone();
                candidate
                    .apply(
                        DockCommand::ResizeColumn {
                            column: resize_column,
                            width: if alternate_preview_width { 279.0 } else { 281.0 },
                        },
                        &metrics,
                    )
                    .expect("benchmark resize candidate must be valid");
                std::hint::black_box(candidate);
            },
        );
    }

    // Use one medium-sized layout and compare disabled history, the default-sized
    // bounded Vec and a deliberately large saturated history. Fill history before
    // timing, so the latter two runs include the steady-state push/remove path.
    for limit in [0usize, 100, 1_000] {
        let (mut layout, column) = history_benchmark_layout(3, 4, 4, 2, 4);
        layout
            .validate(&metrics)
            .expect("history benchmark layout must be valid");
        let nodes = layout.nodes().count();
        let panels = layout.panel_instances().count();
        let dock_areas = layout.dock_areas().count();
        let floating_windows = layout.floating_groups().count();
        let mut history = UndoRedoHistory::with_limit(limit);

        for index in 0..limit {
            let width = if index % 2 == 0 { 310.0 } else { 320.0 };
            history
                .apply(
                    &mut layout,
                    DockCommand::ResizeColumn { column, width },
                    &metrics,
                )
                .expect("history prefill command must succeed");
        }
        assert_eq!(history.undo_len(), limit);

        let iterations = if limit >= 1_000 { 250 } else { 1_000 };
        let mut index = limit;
        report_history_benchmark(
            &format!("medium/saturated-history-limit-{limit}"),
            nodes,
            panels,
            dock_areas,
            floating_windows,
            iterations,
            || {
                let width = if index % 2 == 0 { 310.0 } else { 320.0 };
                index += 1;
                std::hint::black_box(
                    history
                        .apply(
                            &mut layout,
                            DockCommand::ResizeColumn { column, width },
                            &metrics,
                        )
                        .expect("history benchmark command must succeed"),
                );
            },
        );
        assert_eq!(history.undo_len(), limit);
    }
}

#[test]
fn undo_redo_restores_each_persistent_layout_dimension() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    assert_eq!(fixture.layout.validate(&metrics), Ok(()));
    let mut history = UndoRedoHistory::new();

    let cases = vec![
        DockCommand::ResizeColumn {
            column: fixture.column,
            width: 320.0,
        },
        DockCommand::RestoreGroupRatios {
            column: fixture.column,
            ratios: vec![0.3, 0.7],
        },
        DockCommand::ResizeGroups {
            column: fixture.column,
            boundary_index: 0,
            delta: 18.0,
            available_height: 900.0,
        },
        DockCommand::ActivateTab {
            group: fixture.group_a,
            panel: fixture.color,
        },
        DockCommand::ReorderTab {
            group: fixture.group_a,
            panel: fixture.color,
            to_index: 0,
        },
        DockCommand::CollapseGroup {
            group: fixture.group_a,
            collapsed: true,
        },
        DockCommand::SetColumnMode {
            column: fixture.column,
            mode: ColumnMode::IconsOnly,
        },
        DockCommand::MovePanel {
            panel: fixture.properties,
            target: DropTarget::Tabs {
                group: fixture.group_a,
                index: 2,
            },
        },
        DockCommand::Float {
            node: fixture.group_b,
            bounds: Rect::new(Point::new(180.0, 120.0), Size::new(440.0, 340.0)),
        },
        DockCommand::MoveFloating {
            group: fixture.floating_a,
            origin: Point::new(150.0, 160.0),
        },
        DockCommand::ResizeFloating {
            group: fixture.floating_a,
            size: Size::new(500.0, 350.0),
        },
        DockCommand::RaiseFloating {
            group: fixture.floating_a,
        },
        DockCommand::DockFloating {
            group: fixture.floating_a,
            target: DropTarget::Tabs {
                group: fixture.group_a,
                index: 2,
            },
        },
        DockCommand::SetChrome {
            mode: ChromeMode::DockHidden,
        },
    ];

    for command in cases {
        assert_round_trip(&mut fixture.layout, &mut history, command, &metrics);
        assert_eq!(fixture.layout.validate(&metrics), Ok(()));
    }
}

#[test]
fn no_ops_failures_and_transient_flyouts_do_not_create_history() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::new();

    let version = fixture.layout.version();
    let outcome = history
        .apply(
            &mut fixture.layout,
            DockCommand::SetChrome {
                mode: ChromeMode::Visible,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(outcome.new_version, version);
    assert_eq!(history.undo_len(), 0);
    assert!(!history.can_undo());

    assert!(
        history
            .apply(
                &mut fixture.layout,
                DockCommand::ClosePanel {
                    panel: PanelId::null(),
                },
                &metrics,
            )
            .is_err()
    );
    assert_eq!(history.undo_len(), 0);
    assert!(!history.can_undo());

    history
        .apply(
            &mut fixture.layout,
            DockCommand::SetFlyoutAutoCollapse { enabled: false },
            &metrics,
        )
        .unwrap();
    assert_eq!(history.undo_len(), 0);

    fixture
        .layout
        .apply(
            DockCommand::SetColumnMode {
                column: fixture.column,
                mode: ColumnMode::IconsOnly,
            },
            &metrics,
        )
        .unwrap();
    history.clear();
    let version = fixture.layout.version();

    history
        .apply(
            &mut fixture.layout,
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.layers,
            },
            &metrics,
        )
        .unwrap();
    assert!(fixture.layout.flyout().is_some());
    assert_eq!(fixture.layout.version(), version);
    assert!(!history.can_undo());

    history
        .apply(&mut fixture.layout, DockCommand::CloseFlyout, &metrics)
        .unwrap();
    assert!(fixture.layout.flyout().is_none());
    assert!(!history.can_undo());
}

#[test]
fn zero_history_limit_preserves_apply_outcomes_errors_and_state_without_history() {
    let metrics = DockMetrics::default();
    let fixture = fixture();
    let mut history_layout = fixture.layout.clone();
    let mut direct_layout = fixture.layout;
    let mut history = UndoRedoHistory::with_limit(0);

    // A persistent mutation must have the same outcome and state whether it is
    // routed through disabled session history or applied directly to Layout.
    let resize = DockCommand::ResizeColumn {
        column: fixture.column,
        width: 320.0,
    };
    let history_outcome = history
        .apply(&mut history_layout, resize.clone(), &metrics)
        .unwrap();
    let direct_outcome = direct_layout.apply(resize, &metrics).unwrap();
    assert_eq!(history_outcome, direct_outcome);
    assert_eq!(persistent_shape(&history_layout), persistent_shape(&direct_layout));
    assert_eq!(history_layout.version(), direct_layout.version());

    assert_eq!(history.undo_len(), 0);
    assert_eq!(history.redo_len(), 0);
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert!(!history.undo(&mut history_layout, &metrics).unwrap());
    assert!(!history.redo(&mut history_layout, &metrics).unwrap());

    // A rejected command must report the same typed error and leave the
    // already-mutated layout unchanged on both paths.
    let before_failure = persistent_shape(&history_layout);
    let before_failure_version = history_layout.version();
    let invalid_command = DockCommand::ClosePanel {
        panel: PanelId::null(),
    };
    let history_error = history.apply(&mut history_layout, invalid_command.clone(), &metrics);
    let direct_error = direct_layout.apply(invalid_command, &metrics);

    assert_eq!(history_error, direct_error);
    assert!(history_error.is_err());
    assert_eq!(persistent_shape(&history_layout), before_failure);
    assert_eq!(history_layout.version(), before_failure_version);
    assert_eq!(persistent_shape(&history_layout), persistent_shape(&direct_layout));
    assert_eq!(history_layout.version(), direct_layout.version());
    assert_eq!(history.undo_len(), 0);
    assert_eq!(history.redo_len(), 0);
}

#[test]
fn undo_clears_transient_flyout_and_preserves_session_preference() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::new();

    history
        .apply(
            &mut fixture.layout,
            DockCommand::SetColumnMode {
                column: fixture.column,
                mode: ColumnMode::IconsOnly,
            },
            &metrics,
        )
        .unwrap();
    history.clear();
    fixture
        .layout
        .apply(
            DockCommand::SetFlyoutAutoCollapse { enabled: false },
            &metrics,
        )
        .unwrap();
    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.layers,
            },
            &metrics,
        )
        .unwrap();
    history
        .apply(
            &mut fixture.layout,
            DockCommand::SetChrome {
                mode: ChromeMode::AllHidden,
            },
            &metrics,
        )
        .unwrap();

    // SetChrome does not itself mutate the transient flyout selection, but
    // checkpoints omit it and undo must not reopen it.
    assert!(fixture.layout.flyout().is_some());
    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert!(fixture.layout.flyout().is_none());
    assert!(!fixture.layout.flyout_auto_collapse_enabled());
    assert_eq!(fixture.layout.chrome(), ChromeMode::Visible);
}

#[test]
fn narrow_canvas_measurements_can_be_updated_without_blocking_commands() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let narrow = Rect::new(Point::ZERO, Size::new(100.0, 600.0));
    assert!(fixture.layout.update_canvas_bounds(Some(narrow)).unwrap());
    assert_eq!(fixture.layout.canvas().last_known_bounds(), Some(narrow));
    assert_eq!(fixture.layout.validate(&metrics), Ok(()));
    assert!(fixture.layout.apply(DockCommand::SetChrome { mode: ChromeMode::DockHidden }, &metrics).is_ok());
    assert_eq!(fixture.layout.update_canvas_bounds(Some(narrow)), Ok(false));
}

#[test]
fn undo_history_discards_checkpoints_when_used_with_a_replaced_layout() {
    let metrics = DockMetrics::default();
    let mut original = fixture();
    let mut history = UndoRedoHistory::new();
    history.apply(&mut original.layout, DockCommand::ResizeColumn { column: original.column, width: 320.0 }, &metrics).unwrap();
    assert!(history.can_undo());

    let mut replacement = fixture();
    assert!(!history.undo(&mut replacement.layout, &metrics).unwrap());
    assert_eq!(history.undo_len(), 0);
    assert_eq!(history.redo_len(), 0);
    assert_eq!(replacement.layout.version(), 0);

    history.apply(&mut replacement.layout, DockCommand::ResizeColumn { column: replacement.column, width: 330.0 }, &metrics).unwrap();
    assert!(history.undo(&mut replacement.layout, &metrics).unwrap());
    assert_eq!(history.undo_len(), 0);
}

#[test]
fn undo_preserves_panel_runtime_size_changed_after_checkpoint() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    fixture.layout.apply(DockCommand::SetColumnMode { column: fixture.column, mode: ColumnMode::IconsOnly }, &metrics).unwrap();
    fixture.layout.apply(DockCommand::OpenFlyout { column: fixture.column, panel: fixture.layers }, &metrics).unwrap();
    fixture.layout.apply(DockCommand::ResizeFlyout { panel: fixture.layers, width: 320.0 }, &metrics).unwrap();
    let mut history = UndoRedoHistory::new();
    history.apply(&mut fixture.layout, DockCommand::SetChrome { mode: ChromeMode::DockHidden }, &metrics).unwrap();
    fixture.layout.apply(DockCommand::ResizeFlyout { panel: fixture.layers, width: 460.0 }, &metrics).unwrap();

    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    let panel = fixture.layout.panel_instance(fixture.layers).unwrap();
    assert_eq!(panel.state.last_known_size(), None);
    assert_eq!(fixture.layout.flyout_width(&panel.key), Some(460.0));
}

#[test]
fn new_persistent_command_after_undo_discards_redo_branch() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::new();

    history
        .apply(
            &mut fixture.layout,
            DockCommand::ResizeColumn {
                column: fixture.column,
                width: 320.0,
            },
            &metrics,
        )
        .unwrap();
    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert!(history.can_redo());

    history
        .apply(
            &mut fixture.layout,
            DockCommand::ResizeColumn {
                column: fixture.column,
                width: 340.0,
            },
            &metrics,
        )
        .unwrap();
    assert!(!history.can_redo());
    assert_eq!(history.undo_len(), 1);
}

#[test]
fn undo_and_redo_never_revive_stale_panel_ids() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::new();
    let panel_key = key("fresh-history-panel");

    history
        .apply(
            &mut fixture.layout,
            DockCommand::OpenPanel {
                key: panel_key.clone(),
                constraints: constraints(),
                target: Some(DropTarget::Tabs {
                    group: fixture.group_a,
                    index: 2,
                }),
            },
            &metrics,
        )
        .unwrap();

    let old_id = fixture
        .layout
        .panel_instances()
        .find(|(_, instance)| instance.key == panel_key)
        .map(|(id, _)| id)
        .expect("opened panel instance must exist");
    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert!(fixture.layout.panel_instance(old_id).is_none());

    assert!(history.redo(&mut fixture.layout, &metrics).unwrap());
    let new_id = fixture
        .layout
        .panel_instances()
        .find(|(_, instance)| instance.key == panel_key)
        .map(|(id, _)| id)
        .expect("redone panel instance must exist");
    assert_ne!(old_id, new_id, "redo must allocate a fresh runtime key");
    assert!(fixture.layout.panel_instance(old_id).is_none());
}

#[test]
fn failed_undo_keeps_layout_and_history_entry_unchanged() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::new();
    history
        .apply(
            &mut fixture.layout,
            DockCommand::ResizeColumn {
                column: fixture.column,
                width: 320.0,
            },
            &metrics,
        )
        .unwrap();

    let current = persistent_shape(&fixture.layout);
    let current_version = fixture.layout.version();
    let mut invalid_metrics = metrics;
    invalid_metrics.min_canvas_width = f32::NAN;
    assert!(history.undo(&mut fixture.layout, &invalid_metrics).is_err());
    assert_eq!(persistent_shape(&fixture.layout), current);
    assert_eq!(fixture.layout.version(), current_version);
    assert_eq!(history.undo_len(), 1);

    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert!(!history.can_undo());
}

#[test]
fn history_limit_is_bounded() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let mut history = UndoRedoHistory::with_limit(2);

    for width in [310.0, 320.0, 330.0] {
        history
            .apply(
                &mut fixture.layout,
                DockCommand::ResizeColumn {
                    column: fixture.column,
                    width,
                },
                &metrics,
            )
            .unwrap();
    }

    assert_eq!(history.undo_len(), 2);
    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert!(!history.can_undo());
}


#[test]
fn resize_groups_with_tab_chrome_supports_history_undo_and_redo_after_clamping() {
    let metrics = DockMetrics {
        header_height: 36.0,
        tab_strip_height: 44.0,
        ..DockMetrics::default()
    };
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, panel_a) = builder.panel(
        key("history-chrome-resize-a"),
        PanelConstraints::new(
            Size::new(120.0, 120.0),
            Size::new(200.0, 180.0),
            None,
            false,
        ),
    );
    let (_, panel_b) = builder.panel(
        key("history-chrome-resize-b"),
        PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(200.0, 120.0),
            None,
            false,
        ),
    );
    let group_a = builder.tabs(vec![panel_a], 0, false);
    let group_b = builder.tabs(vec![panel_b], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group_a, group_b],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 504.0));
    let group_heights = |layout: &Layout| {
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        let height_a = rects
            .groups
            .iter()
            .find(|item| item.group == group_a)
            .unwrap()
            .rect
            .size
            .height;
        let height_b = rects
            .groups
            .iter()
            .find(|item| item.group == group_b)
            .unwrap()
            .rect
            .size
            .height;
        (height_a, height_b)
    };
    let assert_heights = |layout: &Layout, expected_a: f32, expected_b: f32| {
        let (height_a, height_b) = group_heights(layout);
        assert!((height_a - expected_a).abs() < 1.0e-3);
        assert!((height_b - expected_b).abs() < 1.0e-3);
    };

    let mut history = UndoRedoHistory::new();
    let before = persistent_shape(&layout);
    assert_heights(&layout, 250.0, 250.0);
    let outcome = history
        .apply(
            &mut layout,
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 500.0,
                available_height: 500.0,
            },
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        outcome.actual_value,
        Some(crate::CommandValue::GroupBoundaryDelta(delta)) if (delta - 90.0).abs() < 1.0e-3
    ));
    let after = persistent_shape(&layout);
    assert_ne!(after, before);
    assert_heights(&layout, 340.0, 160.0);
    assert!(history.can_undo());

    assert!(history.undo(&mut layout, &metrics).unwrap());
    assert_eq!(persistent_shape(&layout), before);
    assert_heights(&layout, 250.0, 250.0);
    assert!(history.can_redo());

    assert!(history.redo(&mut layout, &metrics).unwrap());
    assert_eq!(persistent_shape(&layout), after);
    assert_heights(&layout, 340.0, 160.0);
    assert_eq!(layout.validate(&metrics), Ok(()));
}
