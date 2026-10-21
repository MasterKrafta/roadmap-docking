//! C7.1 regression coverage for CollapseGroup semantics and geometry.

use crate::layout_builder::LayoutBuilder;
use crate::{
    ColumnMode, DockCommand, DockMetrics, DockSide, Layout, Node, NodeId, PanelConstraints,
    PanelId, PanelKey, Point, Profile, Rect, Size,
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

fn layout_with_tabs() -> (Layout, NodeId, NodeId, PanelId, PanelId, PanelId) {
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (first_panel, first_node) = builder.panel(key("collapse-first"), constraints());
    let (active_panel, active_node) = builder.panel(key("collapse-active"), constraints());
    let (other_panel, other_node) = builder.panel(key("collapse-other"), constraints());

    let group = builder.tabs(vec![first_node, active_node], 1, false);
    let other_group = builder.tabs(vec![other_node], 0, false);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![group, other_group],
        vec![0.25, 0.75],
    );
    builder.main_area(DockSide::Left, column);

    (builder.build(), group, column, first_panel, active_panel, other_panel)
}

#[test]
fn collapse_and_expand_preserve_tabs_active_index_and_saved_group_ratios() {
    let metrics = DockMetrics::default();
    let (mut layout, group, column, _, active_panel, _) = layout_with_tabs();

    let (children_before, active_index_before) = match layout.node(group).unwrap() {
        Node::Tabs(tabs) => (tabs.children.clone(), tabs.active_index),
        other => panic!("expected Tabs group, found {other:?}"),
    };
    assert_eq!(active_index_before, 1);
    assert_eq!(
        match layout.node(column).unwrap() {
            Node::Column(column) => column.group_ratios.clone(),
            other => panic!("expected Column, found {other:?}"),
        },
        vec![0.25, 0.75],
    );

    let version_before = layout.version();
    let collapsed = layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(collapsed.new_version, version_before + 1);
    assert_eq!(
        collapsed.inverse,
        Some(DockCommand::CollapseGroup {
            group,
            collapsed: false,
        })
    );
    match layout.node(group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(tabs.collapsed);
            assert_eq!(tabs.children, children_before);
            assert_eq!(tabs.active_index, active_index_before);
            assert_eq!(tabs.active_child(), Some(children_before[active_index_before]));
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
    assert_eq!(
        match layout.node(column).unwrap() {
            Node::Column(column) => column.group_ratios.clone(),
            other => panic!("expected Column, found {other:?}"),
        },
        vec![0.25, 0.75],
        "collapsing must not overwrite the last expanded ratio distribution",
    );

    let collapsed_version = layout.version();
    let repeated = layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(repeated.new_version, collapsed_version);
    assert_eq!(layout.version(), collapsed_version);
    assert!(repeated.inverse.is_none());

    let expanded = layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: false,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(expanded.new_version, collapsed_version + 1);
    assert_eq!(
        expanded.inverse,
        Some(DockCommand::CollapseGroup {
            group,
            collapsed: true,
        })
    );
    match layout.node(group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(!tabs.collapsed);
            assert_eq!(tabs.children, children_before);
            assert_eq!(tabs.active_index, active_index_before);
            assert_eq!(tabs.active_child(), Some(children_before[active_index_before]));
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
    assert_eq!(
        match layout.node(column).unwrap() {
            Node::Column(column) => column.group_ratios.clone(),
            other => panic!("expected Column, found {other:?}"),
        },
        vec![0.25, 0.75],
    );
    assert!(layout.node(children_before[active_index_before]).is_some());
    assert!(matches!(
        layout.node(children_before[active_index_before]),
        Some(Node::Panel(panel)) if panel.panel_id == active_panel
    ));
}

#[test]
fn collapsed_geometry_keeps_tab_strip_and_headers_but_hides_panel_content() {
    let metrics = DockMetrics::default();
    let (mut layout, group, _, first_panel, active_panel, other_panel) = layout_with_tabs();
    let viewport = viewport(1200.0, 800.0);

    let expanded_rects = layout.compute_rects(viewport, &metrics).unwrap();
    assert!(expanded_rects.panels.iter().any(|panel| panel.panel == active_panel));

    let children_before = match layout.node(group).unwrap() {
        Node::Tabs(tabs) => tabs.children.clone(),
        other => panic!("expected Tabs group, found {other:?}"),
    };

    layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();

    let rects = layout.compute_rects(viewport, &metrics).unwrap();
    let group_rect = rects.groups.iter().find(|item| item.group == group).unwrap().rect;
    let strip = rects.tab_strips.iter().find(|item| item.group == group).unwrap();
    let collapsed_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);

    assert_eq!(group_rect.size.height, collapsed_height);
    assert_eq!(strip.rect, group_rect);
    let headers: Vec<_> = rects
        .tab_headers
        .iter()
        .filter(|header| header.group == group)
        .map(|header| (header.panel, header.index))
        .collect();
    assert_eq!(headers, vec![(first_panel, 0), (active_panel, 1)]);

    assert!(
        rects.panels.iter().all(|panel| panel.panel != first_panel && panel.panel != active_panel),
        "collapsed tabs must not emit visible content rectangles for their child panels",
    );
    assert!(rects.panels.iter().any(|panel| panel.panel == other_panel));

    match layout.node(group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(tabs.collapsed);
            assert_eq!(tabs.children, children_before);
            assert_eq!(tabs.active_index, 1);
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
}


#[test]
fn reexpand_after_resize_and_reorder_restores_saved_weights_with_current_viewport_minima() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let mut groups = Vec::new();
    for name in ["restore-a", "restore-b", "restore-c"] {
        let (_, panel_node) = builder.panel(key(name), constraints());
        groups.push(builder.tabs(vec![panel_node], 0, false));
    }
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        groups.clone(),
        vec![0.2, 0.3, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::CollapseGroup {
                group: groups[0],
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();

    // available_height excludes the two splitter gaps in this three-group stack.
    layout
        .apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 1,
                delta: 30.0,
                available_height: 692.0,
            },
            &metrics,
        )
        .unwrap();

    let resized_weights = match layout.node(column).unwrap() {
        Node::Column(column) => column.group_ratios.clone(),
        other => panic!("expected Column, found {other:?}"),
    };
    assert!((resized_weights[0] - 0.2).abs() < 1.0e-4);
    assert!(resized_weights[1] > 0.3);
    assert!(resized_weights[2] < 0.5);
    assert!(matches!(
        layout.node(groups[0]),
        Some(Node::Tabs(tabs)) if tabs.collapsed
    ));

    // Reordering the collapsed group must move its ratio with it; its state stays
    // on the group node rather than following a positional slot.
    layout
        .apply(
            DockCommand::MoveGroup {
                group: groups[0],
                column,
                index: 3,
            },
            &metrics,
        )
        .unwrap();

    let reordered_weights = match layout.node(column).unwrap() {
        Node::Column(column) => {
            assert_eq!(column.groups, vec![groups[1], groups[2], groups[0]]);
            column.group_ratios.clone()
        }
        other => panic!("expected Column, found {other:?}"),
    };
    assert!((reordered_weights[0] - resized_weights[1]).abs() < 1.0e-4);
    assert!((reordered_weights[1] - resized_weights[2]).abs() < 1.0e-4);
    assert!((reordered_weights[2] - resized_weights[0]).abs() < 1.0e-4);
    assert!(matches!(
        layout.node(groups[0]),
        Some(Node::Tabs(tabs)) if tabs.collapsed
    ));

    layout
        .apply(
            DockCommand::CollapseGroup {
                group: groups[0],
                collapsed: false,
            },
            &metrics,
        )
        .unwrap();

    let rects = layout
        .compute_rects(viewport(1200.0, 600.0), &metrics)
        .unwrap();
    let height_for = |group| {
        rects
            .groups
            .iter()
            .find(|rect| rect.group == group)
            .unwrap()
            .rect
            .size
            .height
    };
    let available_group_height = 600.0 - 2.0 * metrics.splitter_thickness;
    let rendered_total = groups.iter().map(|group| height_for(*group)).sum::<f32>();
    assert!((rendered_total - available_group_height).abs() < 1.0e-3);

    // At this viewport all groups can honor their minima, so the restored
    // distribution should track the adjusted weights rather than the old pixels.
    for (group, weight) in [
        (groups[1], reordered_weights[0]),
        (groups[2], reordered_weights[1]),
        (groups[0], reordered_weights[2]),
    ] {
        let expected = available_group_height * weight;
        assert!(
            (height_for(group) - expected).abs() < 1.0e-2,
            "group height should follow retained ratio within current viewport constraints",
        );
    }

    // A smaller current viewport may force the ratios away from their ideal
    // proportions, but must keep each expanded group above its content minimum.
    let small_rects = layout
        .compute_rects(viewport(1200.0, 400.0), &metrics)
        .unwrap();
    let minimum_group_height = metrics
        .min_group_height
        .max(metrics.header_height + metrics.tab_strip_height + constraints().min_size.height);
    let small_height_for = |group| {
        small_rects
            .groups
            .iter()
            .find(|rect| rect.group == group)
            .unwrap()
            .rect
            .size
            .height
    };
    assert!(groups
        .iter()
        .all(|group| small_height_for(*group) >= minimum_group_height - 1.0e-3));
    assert!(
        (groups.iter().map(|group| small_height_for(*group)).sum::<f32>() - 392.0).abs()
            < 1.0e-3
    );
    assert!(layout.validate(&metrics).is_ok());
}

#[test]
fn center_drop_of_panel_expands_collapsed_target_in_photoshop_profile() {
    let metrics = DockMetrics::default();
    let (mut layout, target_group, _, first_panel, active_panel, source_panel) =
        layout_with_tabs();
    let source_node = layout
        .nodes()
        .find_map(|(node, value)| {
            matches!(value, Node::Panel(panel) if panel.panel_id == source_panel)
                .then_some(node)
        })
        .unwrap();

    layout
        .apply(
            DockCommand::CollapseGroup {
                group: target_group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: source_panel,
                target: crate::DropTarget::Tabs {
                    group: target_group,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();

    match layout.node(target_group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(!tabs.collapsed);
            assert_eq!(tabs.children[0], layout.nodes().find_map(|(id, node)|
                matches!(node, Node::Panel(panel) if panel.panel_id == first_panel).then_some(id)
            ).unwrap());
            assert_eq!(tabs.children[1], source_node);
            assert_eq!(tabs.children[2], layout.nodes().find_map(|(id, node)|
                matches!(node, Node::Panel(panel) if panel.panel_id == active_panel).then_some(id)
            ).unwrap());
            assert_eq!(tabs.active_index, 1);
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
    assert!(layout.validate(&metrics).is_ok());
}

#[test]
fn center_drop_of_whole_group_expands_collapsed_target_in_photoshop_profile() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (target_first, target_first_node) =
        builder.panel(key("merge-center-target-first"), constraints());
    let (target_second, target_second_node) =
        builder.panel(key("merge-center-target-second"), constraints());
    let target_group = builder.tabs(
        vec![target_first_node, target_second_node],
        0,
        true,
    );
    let (source_panel, source_node) =
        builder.panel(key("merge-center-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group, source_group],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MergeGroup {
                group: source_group,
                target_group,
                index: 1,
            },
            &metrics,
        )
        .unwrap();

    match layout.node(target_group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(!tabs.collapsed);
            assert_eq!(tabs.children, vec![target_first_node, source_node, target_second_node]);
            assert_eq!(tabs.active_index, 1);
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
    assert!(layout.node(source_group).is_none());
    assert!(layout.panel_instance(target_first).is_some());
    assert!(layout.panel_instance(target_second).is_some());
    assert!(layout.panel_instance(source_panel).is_some());
    assert!(layout.validate(&metrics).is_ok());
}

#[test]
fn self_center_drop_does_not_expand_a_collapsed_group_or_increment_version() {
    let metrics = DockMetrics::default();
    let (mut layout, group, _, _, active_panel, _) = layout_with_tabs();
    layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();
    let version = layout.version();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: active_panel,
                target: crate::DropTarget::Tabs { group, index: 1 },
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.version(), version);
    assert!(matches!(
        layout.node(group),
        Some(Node::Tabs(tabs)) if tabs.collapsed
    ));
}

#[test]
fn generic_profile_does_not_implicitly_expand_collapsed_group_on_center_drop() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, target_node) = builder.panel(key("generic-center-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, true);
    let (source_panel, source_node) =
        builder.panel(key("generic-center-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let root = builder.split(crate::Axis::Y, vec![target_group, source_group], vec![0.5, 0.5]);
    builder.main_area(DockSide::Left, root);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: source_panel,
                target: crate::DropTarget::Tabs { group: target_group, index: 1 },
            },
            &metrics,
        )
        .unwrap();

    assert!(matches!(
        layout.node(target_group),
        Some(Node::Tabs(tabs)) if tabs.collapsed && tabs.children.len() == 2
    ));
    assert!(layout.validate(&metrics).is_ok());
}
