use slotmap::Key;

use crate::layout_builder::LayoutBuilder;
use crate::{
    ChromeMode, ColumnMode, DockCommand, DockMetrics, DockSide, DropTarget, Layout,
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

fn persistent_shape(layout: &Layout) -> String {
    let nodes: Vec<_> = layout.nodes().map(|(id, node)| (id, node.clone())).collect();
    let panels: Vec<_> = layout
        .panel_instances()
        .map(|(id, panel)| (id, panel.clone()))
        .collect();
    let areas: Vec<_> = layout.dock_areas().map(|(id, area)| (id, *area)).collect();
    let main_areas = [
        layout.main_area(DockSide::Left).copied(),
        layout.main_area(DockSide::Right).copied(),
    ];
    let floating: Vec<_> = layout
        .floating_groups_in_paint_order()
        .into_iter()
        .map(|(id, group)| (id, *group))
        .collect();
    let remembered_locations: Vec<_> = layout
        .panel_instances()
        .map(|(_, panel)| (panel.key.clone(), layout.last_dock_location(&panel.key)))
        .collect();

    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        layout.profile(),
        layout.chrome(),
        layout.canvas(),
        nodes,
        panels,
        areas,
        main_areas,
        (floating, remembered_locations),
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
    assert!(layout.version() > after_version, "undo must advance revision");

    assert!(history.redo(layout, metrics).unwrap());
    assert_eq!(persistent_shape(layout), after);
    assert!(layout.version() > after_version, "redo must advance revision");

    assert!(history.undo(layout, metrics).unwrap());
    assert_eq!(persistent_shape(layout), before);
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

    assert!(history
        .apply(
            &mut fixture.layout,
            DockCommand::ClosePanel {
                panel: PanelId::null(),
            },
            &metrics,
        )
        .is_err());
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

    assert!(fixture.layout.flyout().is_none());
    assert!(history.undo(&mut fixture.layout, &metrics).unwrap());
    assert!(fixture.layout.flyout().is_none());
    assert!(!fixture.layout.flyout_auto_collapse_enabled());
    assert_eq!(fixture.layout.chrome(), ChromeMode::Visible);
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
