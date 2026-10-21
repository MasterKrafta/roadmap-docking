use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, Column, ColumnMode, DockCommand, DockMetrics, DockSide, DragDropRequest, DragPayload,
    DragSession, DropTarget, DropZone, FloatingGroupId, Layout, Node, NodeId, PanelConstraints,
    PanelId, PanelKey, Point, Profile, Rect, Size, Tabs,
};

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).expect("test panel key must be valid")
}

fn constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(80.0, 60.0),
        Size::new(180.0, 140.0),
        None,
        false,
    )
}

fn bounds(x: f32, y: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(420.0, 320.0))
}

fn assert_tabs(layout: &Layout, group: NodeId, expected: &Tabs) {
    let Some(Node::Tabs(actual)) = layout.node(group) else {
        panic!("expected group {group:?} to remain a Tabs node");
    };
    assert_eq!(actual, expected);
}

fn tab_parent_of(layout: &Layout, panel_node: NodeId) -> Option<NodeId> {
    layout.nodes().find_map(|(id, node)| match node {
        Node::Tabs(tabs) if tabs.children.contains(&panel_node) => Some(id),
        _ => None,
    })
}

fn floating_root(layout: &Layout, id: FloatingGroupId) -> NodeId {
    let floating = layout.floating_group(id).expect("floating group exists");
    layout.dock_area(floating.area).expect("floating area exists").root
}

#[test]
fn panel_tab_reorders_inside_tabs_without_becoming_a_group_drag() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (first_panel, first_node) = builder.panel(key("reorder-first"), constraints());
    let (_, second_node) = builder.panel(key("reorder-second"), constraints());
    let (_, third_node) = builder.panel(key("reorder-third"), constraints());
    let group = builder.tabs(vec![first_node, second_node, third_node], 2, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: first_panel,
                target: DropTarget::Tabs { group, index: 3 },
            },
            &metrics,
        )
        .unwrap();

    let Node::Tabs(tabs) = layout.node(group).unwrap() else {
        panic!("reordering must preserve the Tabs group");
    };
    assert_eq!(tabs.children, vec![second_node, third_node, first_node]);
    assert_eq!(tabs.active_index, 1);
    assert_eq!(tabs.children[tabs.active_index], third_node);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_joins_another_tabs_group_and_only_that_panel_moves() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("join-moving"), constraints());
    let (_, source_keep_node) = builder.panel(key("join-keep"), constraints());
    let source = builder.tabs(vec![moving_node, source_keep_node], 0, true);
    let (_, target_a) = builder.panel(key("join-target-a"), constraints());
    let (_, target_b) = builder.panel(key("join-target-b"), constraints());
    let target = builder.tabs(vec![target_a, target_b], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source, target],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::Tabs { group: target, index: 1 },
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(
        &layout,
        source,
        &Tabs {
            children: vec![source_keep_node],
            active_index: 0,
            collapsed: true,
        },
    );
    assert_tabs(
        &layout,
        target,
        &Tabs {
            children: vec![target_a, moving_node, target_b],
            active_index: 1,
            collapsed: false,
        },
    );
    assert_eq!(layout.panel_instances().count(), 4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_split_into_a_new_group() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("split-panel"), constraints());
    let (_, source_keep) = builder.panel(key("split-source-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, left_column);

    let (_, target_node) = builder.panel(key("split-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, right_column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::GroupEdge {
                    group: target_group,
                    zone: DropZone::Top,
                },
            },
            &metrics,
        )
        .unwrap();

    let new_group = tab_parent_of(&layout, moving_node).expect("moved panel is wrapped as a new group");
    assert_ne!(new_group, source_group);
    assert_ne!(new_group, target_group);
    assert_tabs(
        &layout,
        new_group,
        &Tabs {
            children: vec![moving_node],
            active_index: 0,
            collapsed: false,
        },
    );
    let Node::Column(column) = layout.node(right_column).unwrap() else {
        panic!("right dock root remains a column for a top/bottom split");
    };
    assert_eq!(column.groups, vec![new_group, target_group]);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_become_a_new_main_dock_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("column-panel"), constraints());
    let (_, source_keep) = builder.panel(key("column-source-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, left_column);

    let (_, target_node) = builder.panel(key("column-target"), constraints());
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
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::MainColumn { side: DockSide::Right, index: 0 },
            },
            &metrics,
        )
        .unwrap();

    let Some(Node::Split(split)) = layout.main_area(DockSide::Right).and_then(|area| layout.node(area.root)) else {
        panic!("adding a new column beside an existing one forms a horizontal split");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 2);
    let Node::Column(new_column) = layout.node(split.children[0]).unwrap() else {
        panic!("the inserted root child must be a column");
    };
    let new_group = new_column.groups[0];
    assert_tabs(
        &layout,
        new_group,
        &Tabs {
            children: vec![moving_node],
            active_index: 0,
            collapsed: false,
        },
    );
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_float_without_detaching_its_sibling_tabs() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("float-panel"), constraints());
    let (_, keep_node) = builder.panel(key("float-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, keep_node], 1, true);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::Float {
                node: moving_node,
                bounds: bounds(100.0, 120.0),
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(
        &layout,
        source_group,
        &Tabs {
            children: vec![keep_node],
            active_index: 0,
            collapsed: true,
        },
    );
    assert_eq!(layout.floating_groups().count(), 1);
    let (floating_id, _) = layout.floating_groups().next().unwrap();
    let root = floating_root(&layout, floating_id);
    let Node::Tabs(float_tabs) = layout.node(root).unwrap() else {
        panic!("Photoshop floating panel roots are wrapped in Tabs");
    };
    assert_eq!(float_tabs.children, vec![moving_node]);
    assert!(matches!(layout.node(moving_node), Some(Node::Panel(panel)) if panel.panel_id == moving_panel));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_move_between_floating_groups() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("float-source-moving"), constraints());
    let (_, source_keep) = builder.panel(key("float-source-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep], 0, true);
    let source_float = builder.floating_group(source_group, bounds(80.0, 90.0), 0);

    let (_, target_node) = builder.panel(key("float-destination"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_float = builder.floating_group(target_group, bounds(560.0, 140.0), 1);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::Tabs { group: target_group, index: 1 },
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(
        &layout,
        source_group,
        &Tabs {
            children: vec![source_keep],
            active_index: 0,
            collapsed: true,
        },
    );
    assert_tabs(
        &layout,
        target_group,
        &Tabs {
            children: vec![target_node, moving_node],
            active_index: 1,
            collapsed: false,
        },
    );
    assert_eq!(layout.floating_groups().count(), 2);
    assert_eq!(floating_root(&layout, source_float), source_group);
    assert_eq!(floating_root(&layout, target_float), target_group);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn group_grip_moves_all_tabs_as_one_group_and_preserves_active_and_collapsed_state() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, first_panel) = builder.panel(key("group-first"), constraints());
    let (_, second_panel) = builder.panel(key("group-second"), constraints());
    let (_, third_panel) = builder.panel(key("group-third"), constraints());
    let source_group = builder.tabs(vec![first_panel, second_panel, third_panel], 1, true);
    let original_group = Tabs {
        children: vec![first_panel, second_panel, third_panel],
        active_index: 1,
        collapsed: true,
    };

    let (_, source_sibling_panel) = builder.panel(key("group-source-sibling"), constraints());
    let source_sibling = builder.tabs(vec![source_sibling_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group, source_sibling],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(key("group-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
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
            DockCommand::MoveGroup {
                group: source_group,
                column: target_column,
                index: 1,
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(&layout, source_group, &original_group);
    let Node::Column(source) = layout.node(source_column).unwrap() else {
        panic!("source remains a column after moving one of its groups");
    };
    assert_eq!(source.groups, vec![source_sibling]);
    let Node::Column(target) = layout.node(target_column).unwrap() else {
        panic!("destination remains a column");
    };
    assert_eq!(target.groups, vec![target_group, source_group]);
    assert_eq!(layout.panel_instances().count(), 5);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn group_grip_split_into_a_new_column_preserves_the_group_node_and_state() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("group-split-a"), constraints());
    let (_, p1) = builder.panel(key("group-split-b"), constraints());
    let source_group = builder.tabs(vec![p0, p1], 1, true);
    let original_group = Tabs { children: vec![p0, p1], active_index: 1, collapsed: true };
    let (_, source_keep_panel) = builder.panel(key("group-split-keep"), constraints());
    let source_keep = builder.tabs(vec![source_keep_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group, source_keep],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(key("group-split-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
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
            DockCommand::SplitGroup {
                group: source_group,
                target_group,
                zone: DropZone::Left,
                ratio: 0.4,
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(&layout, source_group, &original_group);
    let Some(Node::Split(split)) = layout.main_area(DockSide::Right).and_then(|area| layout.node(area.root)) else {
        panic!("horizontal group-edge drop creates an X split of columns");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 2);
    let Node::Column(new_column) = layout.node(split.children[0]).unwrap() else {
        panic!("new group column comes before the target column");
    };
    assert_eq!(new_column.groups, vec![source_group]);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn group_grip_floats_the_entire_tabs_node_without_losing_group_state() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("group-float-a"), constraints());
    let (_, p1) = builder.panel(key("group-float-b"), constraints());
    let source_group = builder.tabs(vec![p0, p1], 1, true);
    let original_group = Tabs { children: vec![p0, p1], active_index: 1, collapsed: true };
    let (_, sibling_panel) = builder.panel(key("group-float-sibling"), constraints());
    let sibling = builder.tabs(vec![sibling_panel], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group, sibling],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::Float {
                node: source_group,
                bounds: bounds(180.0, 100.0),
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(&layout, source_group, &original_group);
    let (floating_id, _) = layout.floating_groups().next().unwrap();
    assert_eq!(floating_root(&layout, floating_id), source_group);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn merging_a_group_moves_every_tab_and_activates_its_previous_active_tab() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, s0) = builder.panel(key("merge-source-a"), constraints());
    let (_, s1) = builder.panel(key("merge-source-b"), constraints());
    let (_, s2) = builder.panel(key("merge-source-c"), constraints());
    let source = builder.tabs(vec![s0, s1, s2], 1, false);

    let (_, t0) = builder.panel(key("merge-target-a"), constraints());
    let (_, t1) = builder.panel(key("merge-target-b"), constraints());
    let target = builder.tabs(vec![t0, t1], 0, true);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source, target],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MergeGroup {
                group: source,
                target_group: target,
                index: 1,
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.node(source).is_none(), "a merged group no longer has a separate container");
    assert_tabs(
        &layout,
        target,
        &Tabs {
            children: vec![t0, s0, s1, s2, t1],
            active_index: 2,
            collapsed: true,
        },
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn drop_request_keeps_group_grip_semantics_distinct_from_panel_tab_drag() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("request-group-panel-a"), constraints());
    let (_, p1) = builder.panel(key("request-group-panel-b"), constraints());
    let group = builder.tabs(vec![p0, p1], 1, true);
    let group_node = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, group_node);
    let (_, target_panel) = builder.panel(key("request-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);
    let layout = builder.build();

    let group_payload = DragPayload::Group { group };
    let mut session = DragSession::new(&metrics).unwrap();
    session.pointer_down(group_payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 0.0)).unwrap();
    let target = DropTarget::GroupTabs { group: target_group, index: 1 };
    session.set_drop_target(Some(target)).unwrap();

    let request = session
        .pointer_up(Point::new(12.0, 8.0), Some(target))
        .unwrap()
        .expect("group drop creates a request");
    assert_eq!(
        request,
        DragDropRequest {
            payload: DragPayload::Group { group },
            origin: Point::ZERO,
            point: Point::new(12.0, 8.0),
            target,
        }
    );
    assert!(matches!(request.payload, DragPayload::Group { group: id } if id == group));
    assert!(!matches!(request.payload, DragPayload::Panel { .. }));
    assert!(layout.node(group).is_some());
}
