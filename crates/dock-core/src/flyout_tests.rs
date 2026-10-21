//! C7.3 regression coverage for transient flyout state, geometry and dismissal.

use crate::layout_builder::LayoutBuilder;
use crate::{
    ColumnMode, DockCommand, DockMetrics, DockSide, FlyoutState, HitResult, Layout, Node, NodeId,
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
