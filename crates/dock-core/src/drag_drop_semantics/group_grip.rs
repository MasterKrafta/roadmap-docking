use super::*;
use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DragDropRequest, DragHandle,
    DragPayload, DragSession, DropTarget, DropZone, Node, Point, Profile, Tabs,
};

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
    let original_group = Tabs {
        children: vec![p0, p1],
        active_index: 1,
        collapsed: true,
    };
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
    let Some(Node::Split(split)) = layout
        .main_area(DockSide::Right)
        .and_then(|area| layout.node(area.root))
    else {
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
    let original_group = Tabs {
        children: vec![p0, p1],
        active_index: 1,
        collapsed: true,
    };
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

    assert!(
        layout.node(source).is_none(),
        "a merged group no longer has a separate container"
    );
    assert_tabs(
        &layout,
        target,
        &Tabs {
            children: vec![t0, s0, s1, s2, t1],
            active_index: 2,
            // A successful center merge reveals the target group in Photoshop.
            collapsed: false,
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
    let target = DropTarget::GroupTabs {
        group: target_group,
        index: 1,
    };
    session.set_drop_target(Some(target)).unwrap();

    let request = session
        .pointer_up(Point::new(12.0, 8.0), Some(target))
        .unwrap()
        .expect("group drop creates a request");
    assert_eq!(
        request,
        DragDropRequest {
            payload: DragPayload::Group { group },
            handle: DragHandle::GroupGrip,
            origin: Point::ZERO,
            point: Point::new(12.0, 8.0),
            target: Some(target),
            floating_bounds: None,
        }
    );
    assert!(matches!(request.payload, DragPayload::Group { group: id } if id == group));
    assert!(!matches!(request.payload, DragPayload::Panel { .. }));
    assert!(layout.node(group).is_some());
}
