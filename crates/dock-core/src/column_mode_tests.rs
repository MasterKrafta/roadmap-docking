//! C7.2 regression coverage for expanded and rail column presentations.

use crate::layout_builder::LayoutBuilder;
use crate::{
    ColumnDisplayItem, ColumnMode, DockCommand, DockMetrics, DockSide, HitResult, Layout, Node,
    NodeId, PanelConstraints, PanelId, PanelKey, Point, Profile, Rect, Size,
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

fn viewport(width: f32, height: f32) -> Rect {
    Rect::new(Point::ZERO, Size::new(width, height))
}

struct Fixture {
    layout: Layout,
    column: NodeId,
    groups: Vec<NodeId>,
    panels: Vec<PanelId>,
}

fn fixture() -> Fixture {
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut panels = Vec::new();

    let (a0, a0_node) = builder.panel(key("rail-a0"), constraints());
    let (a1, a1_node) = builder.panel(key("rail-a1"), constraints());
    let group_a = builder.tabs(vec![a0_node, a1_node], 1, false);
    panels.extend([a0, a1]);

    let (b0, b0_node) = builder.panel(key("rail-b0"), constraints());
    let group_b = builder.tabs(vec![b0_node], 0, true);
    panels.push(b0);

    let (c0, c0_node) = builder.panel(key("rail-c0"), constraints());
    let (c1, c1_node) = builder.panel(key("rail-c1"), constraints());
    let group_c = builder.tabs(vec![c0_node, c1_node], 0, false);
    panels.extend([c0, c1]);

    let groups = vec![group_a, group_b, group_c];
    let metrics = DockMetrics::default();
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        groups.clone(),
        vec![0.3, 0.2, 0.5],
    );
    builder.main_area(DockSide::Left, column);

    Fixture {
        layout: builder.build(),
        column,
        groups,
        panels,
    }
}

fn tabs_snapshot(layout: &Layout, column: NodeId) -> Vec<(NodeId, Vec<NodeId>, usize, bool)> {
    let Node::Column(column) = layout.node(column).unwrap() else {
        panic!("expected Column node");
    };
    column
        .groups
        .iter()
        .map(|group| {
            let Node::Tabs(tabs) = layout.node(*group).unwrap() else {
                panic!("expected Tabs node");
            };
            (*group, tabs.children.clone(), tabs.active_index, tabs.collapsed)
        })
        .collect()
}

fn expected_order(fixture: &Fixture) -> Vec<ColumnDisplayItem> {
    vec![
        ColumnDisplayItem::new(fixture.groups[0], fixture.panels[0], 0, 0),
        ColumnDisplayItem::new(fixture.groups[0], fixture.panels[1], 1, 1),
        ColumnDisplayItem::new(fixture.groups[1], fixture.panels[2], 0, 2),
        ColumnDisplayItem::new(fixture.groups[2], fixture.panels[3], 0, 3),
        ColumnDisplayItem::new(fixture.groups[2], fixture.panels[4], 1, 4),
    ]
}

#[test]
fn mode_switches_preserve_column_tree_tab_order_active_tabs_and_collapse_state() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let original_groups = fixture.groups.clone();
    let original_tabs = tabs_snapshot(&fixture.layout, fixture.column);
    let original_order = fixture.layout.column_display_order(fixture.column).unwrap();

    for mode in [
        ColumnMode::IconsAndLabels,
        ColumnMode::IconsOnly,
        ColumnMode::Expanded,
        ColumnMode::IconsOnly,
        ColumnMode::IconsAndLabels,
        ColumnMode::Expanded,
    ] {
        fixture
            .layout
            .apply(
                DockCommand::SetColumnMode {
                    column: fixture.column,
                    mode,
                },
                &metrics,
            )
            .unwrap();

        let Node::Column(column) = fixture.layout.node(fixture.column).unwrap() else {
            panic!("mode switching must preserve the Column node");
        };
        assert_eq!(column.groups, original_groups);
        assert_eq!(
            tabs_snapshot(&fixture.layout, fixture.column),
            original_tabs,
            "mode switching must not rewrite tab order, active indices, or collapsed state",
        );
        assert_eq!(
            fixture.layout.column_display_order(fixture.column).unwrap(),
            original_order,
        );
        assert_eq!(
            column.width,
            match mode {
                ColumnMode::Expanded => metrics.default_column_width,
                ColumnMode::IconsAndLabels => metrics.min_column_width,
                ColumnMode::IconsOnly => metrics.rail_width,
            },
        );
        assert!(fixture.layout.validate(&metrics).is_ok());
    }
}

#[test]
fn both_rail_modes_use_group_then_tab_flattening_for_layout_and_hit_testing() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    let expected = expected_order(&fixture);

    for mode in [ColumnMode::IconsAndLabels, ColumnMode::IconsOnly] {
        fixture
            .layout
            .apply(
                DockCommand::SetColumnMode {
                    column: fixture.column,
                    mode,
                },
                &metrics,
            )
            .unwrap();

        assert_eq!(fixture.layout.column_display_order(fixture.column).unwrap(), expected);
        let rects = fixture
            .layout
            .compute_rects(viewport(1200.0, 500.0), &metrics)
            .unwrap();

        let expected_width = if mode == ColumnMode::IconsOnly {
            metrics.rail_width
        } else {
            metrics.min_column_width
        };
        assert_eq!(rects.columns[0].rect.size.width, expected_width);
        assert!(rects.tab_strips.is_empty());
        assert!(rects.panels.is_empty());
        assert_eq!(rects.tab_headers.len(), expected.len());
        assert_eq!(
            rects
                .tab_headers
                .iter()
                .map(|tab| (tab.group, tab.panel, tab.index))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|item| (item.group, item.panel, item.tab_index))
                .collect::<Vec<_>>(),
        );

        let slot_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);
        for (index, item) in expected.iter().enumerate() {
            let header = rects.tab_headers[index];
            assert_eq!(header.rect.origin.y, index as f32 * slot_height);
            assert_eq!(header.rect.size.height, slot_height);
            assert_eq!(
                fixture
                    .layout
                    .hit_test(
                        Point::new(expected_width / 2.0, (index as f32 + 0.5) * slot_height),
                        &rects,
                        &metrics,
                    )
                    .unwrap(),
                HitResult::TabHeader {
                    group: item.group,
                    panel: item.panel,
                    index: item.tab_index,
                },
            );
        }

        let group_heights: Vec<_> = rects.groups.iter().map(|group| group.rect.size.height).collect();
        assert_eq!(group_heights, vec![2.0 * slot_height, slot_height, 2.0 * slot_height]);
    }
}

#[test]
fn expanded_mode_returns_group_tab_strip_and_active_panel_geometry_again() {
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
            DockCommand::SetColumnMode {
                column: fixture.column,
                mode: ColumnMode::Expanded,
            },
            &metrics,
        )
        .unwrap();

    let rects = fixture
        .layout
        .compute_rects(viewport(1200.0, 500.0), &metrics)
        .unwrap();
    assert_eq!(rects.columns[0].rect.size.width, metrics.default_column_width);
    assert_eq!(rects.tab_strips.len(), 3);
    assert_eq!(rects.tab_headers.len(), 5);
    assert_eq!(
        rects.panels.iter().map(|panel| panel.panel).collect::<Vec<_>>(),
        vec![fixture.panels[1], fixture.panels[3]],
        "only the pre-existing active panel of each expanded group is shown",
    );
}

#[test]
fn rail_modes_can_open_flyouts_and_expanding_closes_the_stale_flyout() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();

    fixture
        .layout
        .apply(
            DockCommand::SetColumnMode {
                column: fixture.column,
                mode: ColumnMode::IconsAndLabels,
            },
            &metrics,
        )
        .unwrap();
    fixture
        .layout
        .apply(
            DockCommand::OpenFlyout {
                column: fixture.column,
                group: fixture.groups[0],
            },
            &metrics,
        )
        .unwrap();
    assert!(fixture.layout.flyout().is_some());
    assert!(fixture.layout.validate(&metrics).is_ok());

    fixture
        .layout
        .apply(
            DockCommand::SetColumnMode {
                column: fixture.column,
                mode: ColumnMode::Expanded,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(fixture.layout.flyout(), None);
    assert!(fixture.layout.validate(&metrics).is_ok());
}

#[test]
fn expanding_a_rail_restores_enough_width_for_panel_minimums() {
    let metrics = DockMetrics::default();
    let wide_constraints = PanelConstraints::new(
        Size::new(420.0, 60.0),
        Size::new(460.0, 140.0),
        Some(Size::new(700.0, 500.0)),
        false,
    );
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, panel_node) = builder.panel(key("rail-wide-panel"), wide_constraints);
    let group = builder.tabs(vec![panel_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    for mode in [ColumnMode::IconsAndLabels, ColumnMode::IconsOnly] {
        layout
            .apply(
                DockCommand::SetColumnMode { column, mode },
                &metrics,
            )
            .unwrap();
        let Node::Column(value) = layout.node(column).unwrap() else {
            panic!("expected Column node");
        };
        assert_eq!(
            value.width,
            if mode == ColumnMode::IconsOnly {
                metrics.rail_width
            } else {
                metrics.min_column_width
            },
        );
    }

    layout
        .apply(
            DockCommand::SetColumnMode {
                column,
                mode: ColumnMode::Expanded,
            },
            &metrics,
        )
        .unwrap();

    let Node::Column(value) = layout.node(column).unwrap() else {
        panic!("expected Column node");
    };
    assert!(value.width >= wide_constraints.min_size.width);
    let rects = layout
        .compute_rects(viewport(1200.0, 500.0), &metrics)
        .unwrap();
    assert_eq!(rects.panels.len(), 1);
    assert_eq!(rects.panels[0].panel, panel);
}

#[test]
fn expanded_width_is_saved_while_rail_widths_are_active() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    fixture.layout.apply(
        DockCommand::ResizeColumn { column: fixture.column, width: 333.0 },
        &metrics,
    ).unwrap();

    for mode in [ColumnMode::IconsOnly, ColumnMode::IconsAndLabels, ColumnMode::IconsOnly] {
        fixture.layout.apply(
            DockCommand::SetColumnMode { column: fixture.column, mode },
            &metrics,
        ).unwrap();
        let Node::Column(column) = fixture.layout.node(fixture.column).unwrap() else {
            panic!("expected Column node");
        };
        assert_eq!(column.width, if mode == ColumnMode::IconsOnly {
            metrics.rail_width
        } else {
            metrics.min_column_width
        });
        assert_eq!(column.last_expanded_width, Some(333.0));
    }
    fixture.layout.apply(
        DockCommand::SetColumnMode { column: fixture.column, mode: ColumnMode::Expanded },
        &metrics,
    ).unwrap();
    let Node::Column(column) = fixture.layout.node(fixture.column).unwrap() else {
        panic!("expected Column node");
    };
    assert_eq!(column.width, 333.0);
    assert_eq!(column.last_expanded_width, Some(333.0));
    assert!(fixture.layout.validate(&metrics).is_ok());
}

#[test]
fn rail_slots_use_flat_icon_indices_and_move_the_existing_panel_node() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    fixture.layout.apply(
        DockCommand::SetColumnMode { column: fixture.column, mode: ColumnMode::IconsOnly },
        &metrics,
    ).unwrap();

    let panel = fixture.panels[1];
    let original_node = fixture.layout.nodes().find_map(|(id, node)| {
        matches!(node, Node::Panel(value) if value.panel_id == panel).then_some(id)
    }).unwrap();
    let node_count = fixture.layout.nodes().count();
    let panel_count = fixture.layout.panel_instances().count();

    fixture.layout.apply(
        DockCommand::MovePanel {
            panel,
            target: crate::DropTarget::RailSlot { column: fixture.column, index: 4 },
        },
        &metrics,
    ).unwrap();

    assert_eq!(fixture.layout.nodes().count(), node_count);
    assert_eq!(fixture.layout.panel_instances().count(), panel_count);
    let matching_nodes = fixture.layout.nodes().filter_map(|(id, node)| {
        matches!(node, Node::Panel(value) if value.panel_id == panel).then_some(id)
    }).collect::<Vec<_>>();
    assert_eq!(matching_nodes, vec![original_node]);
    assert_eq!(
        fixture.layout.column_display_order(fixture.column).unwrap()
            .iter().map(|item| item.panel).collect::<Vec<_>>(),
        vec![fixture.panels[0], fixture.panels[2], fixture.panels[3], panel, fixture.panels[4]],
    );
    assert_eq!(fixture.layout.column_display_order(fixture.column).unwrap()[3].group, fixture.groups[2]);
    assert!(fixture.layout.validate(&metrics).is_ok());
}

#[test]
fn crossing_a_rail_group_boundary_places_the_same_panel_in_a_new_group() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    fixture.layout.apply(
        DockCommand::SetColumnMode { column: fixture.column, mode: ColumnMode::IconsOnly },
        &metrics,
    ).unwrap();
    let panel = fixture.panels[0];
    let panel_node = fixture.layout.nodes().find_map(|(id, node)| {
        matches!(node, Node::Panel(value) if value.panel_id == panel).then_some(id)
    }).unwrap();
    let original_instances = fixture.layout.panel_instances().count();

    fixture.layout.apply(
        DockCommand::MovePanel {
            panel,
            target: crate::DropTarget::RailSlot { column: fixture.column, index: 3 },
        },
        &metrics,
    ).unwrap();

    let Node::Column(column) = fixture.layout.node(fixture.column).unwrap() else {
        panic!("expected Column node");
    };
    assert_eq!(column.groups.len(), 4);
    assert_eq!(column.groups[0], fixture.groups[0]);
    assert_eq!(column.groups[1], fixture.groups[1]);
    assert_eq!(column.groups[3], fixture.groups[2]);
    let new_group = column.groups[2];
    let Node::Tabs(tabs) = fixture.layout.node(new_group).unwrap() else {
        panic!("rail boundary crossing must create a Tabs group");
    };
    assert_eq!(tabs.children, vec![panel_node]);
    assert_eq!(tabs.active_index, 0);
    assert_eq!(fixture.layout.panel_instances().count(), original_instances);
    assert_eq!(fixture.layout.column_display_order(fixture.column).unwrap()
        .iter().filter(|item| item.panel == panel).count(), 1);
    assert!(fixture.layout.validate(&metrics).is_ok());
}

#[test]
fn clicking_a_rail_icon_activates_its_panel_and_opens_the_group_flyout() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    fixture.layout.apply(
        DockCommand::SetColumnMode { column: fixture.column, mode: ColumnMode::IconsOnly },
        &metrics,
    ).unwrap();
    let panel = fixture.panels[0];
    fixture.layout.apply(
        DockCommand::OpenPanelFlyout { column: fixture.column, panel },
        &metrics,
    ).unwrap();

    let Node::Tabs(tabs) = fixture.layout.node(fixture.groups[0]).unwrap() else {
        panic!("expected source Tabs group");
    };
    assert_eq!(tabs.active_index, 0);
    assert_eq!(fixture.layout.flyout().unwrap().column, fixture.column);
    assert_eq!(fixture.layout.flyout().unwrap().group, fixture.groups[0]);
    assert!(fixture.layout.validate(&metrics).is_ok());
}

#[test]
fn reopening_a_singleton_from_the_window_menu_activates_it_and_opens_rail_flyout() {
    let metrics = DockMetrics::default();
    let mut fixture = fixture();
    fixture.layout.apply(
        DockCommand::SetColumnMode { column: fixture.column, mode: ColumnMode::IconsAndLabels },
        &metrics,
    ).unwrap();
    let panel_count = fixture.layout.panel_instances().count();
    let singleton_constraints = PanelConstraints::new(
        Size::new(80.0, 60.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        true,
    );

    fixture.layout.apply(
        DockCommand::OpenPanel {
            key: key("rail-a0"),
            constraints: singleton_constraints,
            target: None,
        },
        &metrics,
    ).unwrap();
    let Node::Tabs(tabs) = fixture.layout.node(fixture.groups[0]).unwrap() else {
        panic!("expected source Tabs group");
    };
    assert_eq!(tabs.active_index, 0);
    assert_eq!(fixture.layout.panel_instances().count(), panel_count);
    assert_eq!(fixture.layout.flyout().unwrap().group, fixture.groups[0]);

    let version = fixture.layout.version();
    fixture.layout.apply(
        DockCommand::OpenPanel {
            key: key("rail-a0"),
            constraints: singleton_constraints,
            target: None,
        },
        &metrics,
    ).unwrap();
    assert_eq!(fixture.layout.version(), version);
    assert_eq!(fixture.layout.panel_instances().count(), panel_count);
}


#[test]
fn nested_generic_rail_groups_can_open_flyouts() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (panel, panel_node) = builder.panel(key("generic-nested-rail"), constraints());
    let group = builder.tabs(vec![panel_node], 0, false);
    let split = builder.split(crate::Axis::Y, vec![group], vec![1.0]);
    let column = builder.column(
        metrics.rail_width,
        ColumnMode::IconsOnly,
        vec![split],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout.apply(
        DockCommand::OpenPanelFlyout { column, panel },
        &metrics,
    ).unwrap();

    assert_eq!(layout.flyout().unwrap().group, group);
    assert_eq!(layout.validate(&metrics), Ok(()));
}
