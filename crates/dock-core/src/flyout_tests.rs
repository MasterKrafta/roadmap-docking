//! C7.3 regression coverage for transient flyout state, geometry and dismissal.

use crate::layout_builder::LayoutBuilder;
use crate::{
    ColumnMode, DockCommand, DockMetrics, DockSide, DragDropAction, DragHandle, DragPayload,
    DragSession, DragState, DropTarget, FlyoutState, HitResult, Layout, Node, NodeId,
    PanelConstraints, PanelId, PanelKey, Point, Profile, Rect, ResizeEdge, Size,
};

fn constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(80.0, 60.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        false,
    )
}

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).unwrap()
}

fn viewport() -> Rect {
    Rect::new(Point::ZERO, Size::new(1200.0, 800.0))
}

struct Fixture {
    layout: Layout,
    column: NodeId,
    group: NodeId,
    first_panel: PanelId,
    second_panel: PanelId,
}

fn fixture_on(side: DockSide) -> Fixture {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (first_panel, first_node) = builder.panel(key("flyout-first"), constraints());
    let (second_panel, second_node) = builder.panel(key("flyout-second"), constraints());
    let group = builder.tabs(vec![first_node, second_node], 1, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(side, column);

    Fixture {
        layout: builder.build(),
        column,
        group,
        first_panel,
        second_panel,
    }
}

fn fixture() -> Fixture {
    fixture_on(DockSide::Left)
}

fn put_in_icon_rail(fixture: &mut Fixture, metrics: &DockMetrics) {
    fixture
        .layout
        .apply(
            DockCommand::SetColumnMode {
                column: fixture.column,
                mode: ColumnMode::IconsOnly,
            },
            metrics,
        )
        .unwrap();
}

fn tree_snapshot(layout: &Layout) -> Vec<(NodeId, Node)> {
    layout.nodes().map(|(id, node)| (id, node.clone())).collect()
}

fn active_index(layout: &Layout, group: NodeId) -> usize {
    match layout.node(group).unwrap() {
        Node::Tabs(tabs) => tabs.active_index,
        _ => panic!("expected Tabs node"),
    }
}

fn assert_transient_outcome(outcome: &crate::CommandOutcome, version: u64) {
    assert!(outcome.changed_nodes.is_empty());
    assert_eq!(outcome.new_version, version);
    assert!(
        outcome.inverse.is_none(),
        "temporary flyout UI state must not create an undo record",
    );
}

#[test]
fn opening_switching_and_explicitly_closing_flyouts_preserves_the_dock_tree() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    put_in_icon_rail(&mut fixture, &metrics);

    let tree_before = tree_snapshot(&fixture.layout);
    let canvas_before = *fixture.layout.canvas();
    let rects_before = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    let version = fixture.layout.version();
    let original_active_index = active_index(&fixture.layout, fixture.group);

    let opened = fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();
    assert_transient_outcome(&opened, version);
    assert_eq!(
        fixture.layout.flyout(),
        Some(FlyoutState::new(fixture.column, fixture.group, fixture.first_panel)),
    );
    let open_rects = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    assert_eq!(open_rects.canvas, rects_before.canvas);
    assert_eq!(open_rects.columns, rects_before.columns);
    let flyout_rect = open_rects.flyout.unwrap();
    assert_eq!(flyout_rect.panel, fixture.first_panel);
    assert_eq!(flyout_rect.rect.size.width, constraints().preferred_size.width);
    assert_eq!(flyout_rect.resize_edge, ResizeEdge::Right);
    assert_eq!(active_index(&fixture.layout, fixture.group), original_active_index);
    assert_eq!(tree_snapshot(&fixture.layout), tree_before);
    assert_eq!(*fixture.layout.canvas(), canvas_before);

    // Clicking another icon replaces the current flyout rather than stacking a second one.
    let switched = fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.second_panel,
            },
            &metrics,
        )
        .unwrap();
    assert_transient_outcome(&switched, version);
    assert_eq!(
        fixture.layout.flyout(),
        Some(FlyoutState::new(fixture.column, fixture.group, fixture.second_panel)),
    );
    let switched_rects = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    assert_eq!(switched_rects.flyout.unwrap().panel, fixture.second_panel);
    assert_eq!(switched_rects.canvas, rects_before.canvas);
    assert_eq!(switched_rects.columns, rects_before.columns);
    assert_eq!(active_index(&fixture.layout, fixture.group), original_active_index);
    assert_eq!(tree_snapshot(&fixture.layout), tree_before);
    assert_eq!(*fixture.layout.canvas(), canvas_before);

    let closed = fixture.layout.apply(DockCommand::CloseFlyout, &metrics).unwrap();
    assert_transient_outcome(&closed, version);
    assert_eq!(fixture.layout.flyout(), None);
    assert_eq!(fixture.layout.compute_rects(viewport(), &metrics).unwrap(), rects_before);
    assert_eq!(tree_snapshot(&fixture.layout), tree_before);
    assert_eq!(*fixture.layout.canvas(), canvas_before);
}

#[test]
fn same_icon_and_passive_dismissal_obey_auto_collapse_policy() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    put_in_icon_rail(&mut fixture, &metrics);
    let version = fixture.layout.version();

    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();
    let toggled_closed = fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();
    assert_transient_outcome(&toggled_closed, version);
    assert_eq!(fixture.layout.flyout(), None);

    let changed_policy = fixture
        .layout
        .apply(DockCommand::SetFlyoutAutoCollapse { enabled: false }, &metrics)
        .unwrap();
    assert_transient_outcome(&changed_policy, version);
    assert!(!fixture.layout.flyout_auto_collapse_enabled());

    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();
    // Clicking the already open icon does not close the flyout when auto-collapse is off.
    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(fixture.layout.flyout().unwrap().panel, fixture.first_panel);

    // Click-away and Escape adapters dispatch DismissFlyout. It is ignored while disabled.
    fixture
        .layout
        .apply(DockCommand::DismissFlyout, &metrics)
        .unwrap();
    assert_eq!(fixture.layout.flyout().unwrap().panel, fixture.first_panel);

    // A different icon still replaces the active flyout when auto-collapse is off.
    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.second_panel,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(fixture.layout.flyout().unwrap().panel, fixture.second_panel);

    // Explicit close always works, irrespective of the policy.
    fixture
        .layout
        .apply(DockCommand::CloseFlyout, &metrics)
        .unwrap();
    assert_eq!(fixture.layout.flyout(), None);
    fixture
        .layout
        .apply(DockCommand::OpenFlyout {
            column: fixture.column,
            panel: fixture.second_panel,
        }, &metrics)
        .unwrap();
    fixture
        .layout
        .apply(DockCommand::SetFlyoutAutoCollapse { enabled: true }, &metrics)
        .unwrap();
    fixture
        .layout
        .apply(DockCommand::DismissFlyout, &metrics)
        .unwrap();
    assert_eq!(fixture.layout.flyout(), None);
    assert!(fixture.layout.flyout_auto_collapse_enabled());
    assert_eq!(fixture.layout.version(), version);
}

#[test]
fn resizing_remembers_panel_specific_width_and_exposes_canvas_facing_hit_target() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    put_in_icon_rail(&mut fixture, &metrics);
    let tree_before = tree_snapshot(&fixture.layout);
    let version = fixture.layout.version();

    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();
    let resized = fixture
        .layout
        .apply(
            DockCommand::ResizeFlyout {
                panel: fixture.first_panel,
                width: 420.0,
            },
            &metrics,
        )
        .unwrap();
    assert_transient_outcome(&resized, version);
    assert_eq!(tree_snapshot(&fixture.layout), tree_before);
    assert_eq!(
        fixture.layout.panel_instance(fixture.first_panel).unwrap().state.last_known_size(),
        Some(Size::new(420.0, constraints().preferred_size.height)),
    );

    let rects = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    let flyout = rects.flyout.unwrap();
    assert_eq!(flyout.panel, fixture.first_panel);
    assert_eq!(flyout.rect.size.width, 420.0);
    assert_eq!(flyout.resize_edge, ResizeEdge::Right);
    let edge_point = Point::new(
        flyout.rect.origin.x + flyout.rect.size.width,
        flyout.rect.origin.y + 100.0,
    );
    assert!(matches!(
        fixture.layout.hit_test(edge_point, &rects, &metrics).unwrap(),
        HitResult::FlyoutResizeEdge { column, panel, edge: ResizeEdge::Right }
            if column == fixture.column && panel == fixture.first_panel
    ));
    let content_point = Point::new(
        flyout.rect.origin.x + flyout.rect.size.width / 2.0,
        flyout.rect.origin.y + 400.0,
    );
    assert!(matches!(
        fixture.layout.hit_test(content_point, &rects, &metrics).unwrap(),
        HitResult::FlyoutPanel { column, group, panel }
            if column == fixture.column && group == fixture.group && panel == fixture.first_panel
    ));

    fixture.layout.apply(DockCommand::CloseFlyout, &metrics).unwrap();
    fixture.layout.apply(
        DockCommand::OpenFlyout {
            column: fixture.column,
            panel: fixture.second_panel,
        },
        &metrics,
    ).unwrap();
    assert_eq!(
        fixture.layout.compute_rects(viewport(), &metrics).unwrap().flyout.unwrap().rect.size.width,
        constraints().preferred_size.width,
        "another panel should start from its own preferred/remembered width",
    );

    fixture.layout.apply(DockCommand::CloseFlyout, &metrics).unwrap();
    fixture.layout.apply(
        DockCommand::OpenFlyout {
            column: fixture.column,
            panel: fixture.first_panel,
        },
        &metrics,
    ).unwrap();
    assert_eq!(
        fixture.layout.compute_rects(viewport(), &metrics).unwrap().flyout.unwrap().rect.size.width,
        420.0,
        "reopening a panel should restore its remembered width",
    );
    assert_eq!(fixture.layout.version(), version);
}

#[test]
fn right_side_flyout_resizes_toward_canvas_from_its_left_edge() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture_on(DockSide::Right);
    put_in_icon_rail(&mut fixture, &metrics);
    fixture.layout.apply(
        DockCommand::OpenFlyout {
            column: fixture.column,
            panel: fixture.first_panel,
        },
        &metrics,
    ).unwrap();

    let rects = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    let flyout = rects.flyout.unwrap();
    assert_eq!(flyout.resize_edge, ResizeEdge::Left);
    let point = Point::new(flyout.rect.origin.x, flyout.rect.origin.y + 120.0);
    assert!(matches!(
        fixture.layout.hit_test(point, &rects, &metrics).unwrap(),
        HitResult::FlyoutResizeEdge { column, panel, edge: ResizeEdge::Left }
            if column == fixture.column && panel == fixture.first_panel
    ));
}

struct FlyoutDragFixture {
    layout: Layout,
    source_column: NodeId,
    source_group: NodeId,
    dragged_panel: PanelId,
    target_group: NodeId,
}

fn flyout_drag_fixture() -> FlyoutDragFixture {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (dragged_panel, dragged_node) = builder.panel(key("flyout-drag-source"), constraints());
    let (_, remaining_node) = builder.panel(key("flyout-drag-remaining"), constraints());
    let source_group = builder.tabs(vec![dragged_node, remaining_node], 0, false);
    let source_column = builder.column(
        metrics.rail_width,
        ColumnMode::IconsOnly,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_node) = builder.panel(key("flyout-drag-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    layout
        .apply(
            DockCommand::OpenFlyout {
                column: source_column,
                panel: dragged_panel,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.validate(&metrics), Ok(()));

    FlyoutDragFixture {
        layout,
        source_column,
        source_group,
        dragged_panel,
        target_group,
    }
}

#[test]
fn flyout_panel_drag_keeps_source_open_until_successful_drop_then_closes_it() {
    let metrics = DockMetrics::default();
    let mut fixture = flyout_drag_fixture();
    let flyout_before = fixture.layout.flyout().unwrap();
    let before = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    let target = DropTarget::Tabs {
        group: fixture.target_group,
        index: 1,
    };
    let mut session = DragSession::new(&metrics).unwrap();
    let origin = Point::new(40.0, 40.0);

    session
        .pointer_down_with_handle(
            DragPayload::Panel { panel: fixture.dragged_panel },
            DragHandle::FlyoutPanel,
            origin,
        )
        .unwrap();
    assert_eq!(fixture.layout.flyout(), Some(flyout_before));

    session.pointer_move(Point::new(45.0, 40.0)).unwrap();
    assert!(matches!(session.state(), DragState::Dragging { .. }));
    assert_eq!(fixture.layout.flyout(), Some(flyout_before));
    let during = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    assert_eq!(during.canvas, before.canvas);
    assert_eq!(during.columns, before.columns);
    assert_eq!(during.flyout, before.flyout);

    let request = session
        .pointer_up(Point::new(500.0, 240.0), Some(target))
        .unwrap()
        .expect("valid release emits exactly one drop request");
    assert_eq!(request.handle, DragHandle::FlyoutPanel);
    assert_eq!(fixture.layout.flyout(), Some(flyout_before));

    let action = fixture.layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MovePanelFromFlyout {
            panel: fixture.dragged_panel,
            target,
        }),
    );
    let DragDropAction::Command(command) = action else {
        panic!("a valid flyout-panel drop must produce one explicit command");
    };
    fixture.layout.apply(command, &metrics).unwrap();

    assert_eq!(fixture.layout.flyout(), None);
    assert_eq!(fixture.layout.version(), 1, "the accepted dock move is one persistent transaction");
    assert!(fixture.layout.validate(&metrics).is_ok());
    let after = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    assert_eq!(after.canvas, before.canvas, "the flyout overlays instead of moving Canvas");
    assert_eq!(after.columns, before.columns, "the flyout overlay does not resize dock columns");
    let Node::Tabs(target_tabs) = fixture.layout.node(fixture.target_group).unwrap() else {
        panic!("target should remain a tab group");
    };
    assert!(target_tabs.children.iter().any(|child| matches!(
        fixture.layout.node(*child),
        Some(Node::Panel(panel)) if panel.panel_id == fixture.dragged_panel
    )));
}

#[test]
fn cancelled_flyout_panel_drag_leaves_the_source_flyout_open() {
    let metrics = DockMetrics::default();
    let mut fixture = flyout_drag_fixture();
    let flyout_before = fixture.layout.flyout();
    let before = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    let mut session = DragSession::new(&metrics).unwrap();
    let origin = Point::new(40.0, 40.0);

    session
        .pointer_down_with_handle(
            DragPayload::Panel { panel: fixture.dragged_panel },
            DragHandle::FlyoutPanel,
            origin,
        )
        .unwrap();
    session.pointer_move(Point::new(45.0, 40.0)).unwrap();
    session.escape().unwrap();

    assert!(matches!(
        session.state(),
        DragState::Cancelled { .. }
    ));
    assert_eq!(fixture.layout.flyout(), flyout_before);
    let after = fixture.layout.compute_rects(viewport(), &metrics).unwrap();
    assert_eq!(after.canvas, before.canvas);
    assert_eq!(after.columns, before.columns);
    assert_eq!(after.flyout, before.flyout);
    assert!(fixture.layout.validate(&metrics).is_ok());
}

#[test]
fn flyout_panel_drop_to_free_area_floats_panel_and_closes_flyout() {
    let metrics = DockMetrics::default();
    let mut fixture = flyout_drag_fixture();
    let mut session = DragSession::new(&metrics).unwrap();

    session
        .pointer_down_with_handle(
            DragPayload::Panel { panel: fixture.dragged_panel },
            DragHandle::FlyoutPanel,
            Point::new(40.0, 40.0),
        )
        .unwrap();
    session.pointer_move(Point::new(45.0, 40.0)).unwrap();
    let mut request = session
        .pointer_up(Point::new(650.0, 340.0), Some(DropTarget::FreeArea))
        .unwrap()
        .unwrap();
    request.floating_bounds = Some(Rect::new(
        Point::new(520.0, 180.0),
        Size::new(420.0, 280.0),
    ));

    let action = fixture.layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::FloatPanelFromFlyout {
            panel: fixture.dragged_panel,
            bounds: request.floating_bounds.unwrap(),
        }),
    );
    let DragDropAction::Command(command) = action else {
        panic!("a valid FreeArea drop must produce one explicit float command");
    };
    fixture.layout.apply(command, &metrics).unwrap();

    assert_eq!(fixture.layout.flyout(), None);
    assert_eq!(fixture.layout.floating_groups().count(), 1);
    assert!(fixture.layout.validate(&metrics).is_ok());
}

#[test]
fn normalization_closes_a_flyout_when_its_selected_panel_is_removed() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    put_in_icon_rail(&mut fixture, &metrics);
    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();
    assert!(fixture.layout.flyout().is_some());

    fixture
        .layout
        .apply(
            DockCommand::ClosePanel {
                panel: fixture.first_panel,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(fixture.layout.flyout(), None);
    assert!(fixture.layout.validate(&metrics).is_ok());
}
