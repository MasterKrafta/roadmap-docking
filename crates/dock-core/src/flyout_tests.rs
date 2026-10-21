//! C7.3 regression coverage for transient flyout state.

use crate::layout_builder::LayoutBuilder;
use crate::{
    ColumnMode, DockCommand, DockMetrics, DockSide, FlyoutState, Layout, Node, NodeId,
    PanelConstraints, PanelId, PanelKey, Point, Profile, Rect, Size,
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

fn fixture() -> Fixture {
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
    builder.main_area(DockSide::Left, column);

    Fixture {
        layout: builder.build(),
        column,
        group,
        first_panel,
        second_panel,
    }
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
    assert!(outcome.inverse.is_none(), "temporary display state must not create an undo record");
}

#[test]
fn opening_switching_and_closing_flyouts_only_changes_temporary_display_state() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
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
    assert_eq!(active_index(&fixture.layout, fixture.group), original_active_index);
    assert_eq!(tree_snapshot(&fixture.layout), tree_before);
    assert_eq!(*fixture.layout.canvas(), canvas_before);
    assert_eq!(fixture.layout.compute_rects(viewport(), &metrics).unwrap(), rects_before);

    // Opening a different icon replaces the one flyout instead of stacking another.
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
    assert_eq!(active_index(&fixture.layout, fixture.group), original_active_index);
    assert_eq!(tree_snapshot(&fixture.layout), tree_before);
    assert_eq!(*fixture.layout.canvas(), canvas_before);
    assert_eq!(fixture.layout.compute_rects(viewport(), &metrics).unwrap(), rects_before);

    let closed = fixture.layout.apply(DockCommand::CloseFlyout, &metrics).unwrap();
    assert_transient_outcome(&closed, version);
    assert_eq!(fixture.layout.flyout(), None);
    assert_eq!(tree_snapshot(&fixture.layout), tree_before);
    assert_eq!(*fixture.layout.canvas(), canvas_before);
    assert_eq!(fixture.layout.compute_rects(viewport(), &metrics).unwrap(), rects_before);

    // Closing an already closed flyout is a repeatable no-op.
    let close_again = fixture.layout.apply(DockCommand::CloseFlyout, &metrics).unwrap();
    assert_transient_outcome(&close_again, version);
    assert_eq!(fixture.layout.flyout(), None);
}

#[test]
fn normalization_closes_a_flyout_when_its_selected_panel_is_removed() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
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
