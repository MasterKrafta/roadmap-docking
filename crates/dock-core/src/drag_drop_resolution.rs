use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DragDropAction, DragDropRequest,
    DragHandle, DragPayload, DragSession, DragSessionError, DropSource, DropTarget,
    FloatingGroupId, Layout, Node, NodeId, PanelConstraints, PanelKey, Point, Profile, Rect, Size,
    Tabs,
};

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).unwrap()
}

fn constraints() -> PanelConstraints {
    PanelConstraints::new(Size::new(80.0, 60.0), Size::new(180.0, 140.0), None, false)
}

fn bounds(x: f32, y: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(420.0, 320.0))
}

fn floating_layout() -> (Layout, FloatingGroupId, NodeId) {
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("floating-root-a"), constraints());
    let (_, p1) = builder.panel(key("floating-root-b"), constraints());
    let group = builder.tabs(vec![p0, p1], 1, true);
    let floating = builder.floating_group(group, bounds(100.0, 200.0), 1);
    (builder.build(), floating, group)
}

fn group_layout() -> (Layout, NodeId, NodeId, NodeId) {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("group-source-a"), constraints());
    let (_, p1) = builder.panel(key("group-source-b"), constraints());
    let source = builder.tabs(vec![p0, p1], 1, true);
    let (_, target_panel) = builder.panel(key("group-destination"), constraints());
    let target = builder.tabs(vec![target_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);
    (builder.build(), source, target, target_column)
}

fn tabs(layout: &Layout, id: NodeId) -> &Tabs {
    match layout.node(id).unwrap() {
        Node::Tabs(tabs) => tabs,
        other => panic!("expected Tabs node, got {other:?}"),
    }
}

#[test]
fn floating_title_and_capture_handles_are_explicit_and_payload_checked() {
    let metrics = DockMetrics::default();
    let (_, floating_id, _) = floating_layout();
    let mut session = DragSession::new(&metrics).unwrap();

    assert_eq!(
        session.pointer_down_with_handle(
            DragPayload::FloatingGroup { group: floating_id },
            DragHandle::GroupGrip,
            Point::ZERO,
        ),
        Err(DragSessionError::PayloadHandleMismatch)
    );
    assert_eq!(session.state(), crate::DragState::Idle);

    session
        .pointer_down_with_handle(
            DragPayload::FloatingGroup { group: floating_id },
            DragHandle::FloatingCapture,
            Point::ZERO,
        )
        .unwrap();
    session.pointer_move(Point::new(8.0, 0.0)).unwrap();
    assert_eq!(session.set_drop_target(None), Ok(()));
    assert!(session.preview().unwrap().target.is_none());
}

#[test]
fn floating_title_drag_resolves_to_window_move_without_any_drop_target() {
    let metrics = DockMetrics::default();
    let (mut layout, floating_id, _) = floating_layout();
    let version = layout.version();
    let payload = DragPayload::FloatingGroup { group: floating_id };
    let mut session = DragSession::new(&metrics).unwrap();

    session
        .pointer_down_with_handle(payload, DragHandle::FloatingTitle, Point::new(10.0, 20.0))
        .unwrap();
    session.pointer_move(Point::new(14.0, 20.0)).unwrap();
    assert_eq!(
        session.set_drop_target(Some(DropTarget::FreeArea)),
        Err(DragSessionError::NotDockingDrag)
    );
    let request = session
        .pointer_up(Point::new(30.0, 25.0), None)
        .unwrap()
        .unwrap();

    assert_eq!(request.handle, DragHandle::FloatingTitle);
    assert_eq!(request.target, None);
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveFloating {
            group: floating_id,
            origin: Point::new(120.0, 205.0),
        })
    );

    let DragDropAction::Command(command) = action else {
        panic!("window moved");
    };
    let outcome = layout.apply(command, &metrics).unwrap();
    assert_eq!(outcome.new_version, version + 1);
    assert_eq!(
        layout.floating_group(floating_id).unwrap().bounds.origin,
        Point::new(120.0, 205.0)
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_title_released_at_its_original_point_is_a_noop_without_version_change() {
    let metrics = DockMetrics::default();
    let (mut layout, floating_id, _) = floating_layout();
    let version = layout.version();
    let payload = DragPayload::FloatingGroup { group: floating_id };
    let mut session = DragSession::new(&metrics).unwrap();

    session
        .pointer_down_with_handle(payload, DragHandle::FloatingTitle, Point::new(40.0, 50.0))
        .unwrap();
    session.pointer_move(Point::new(48.0, 50.0)).unwrap();
    let request = session
        .pointer_up(Point::new(40.0, 50.0), None)
        .unwrap()
        .unwrap();

    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        DragDropAction::NoOp
    );
    assert_eq!(layout.version(), version);
    let outcome = layout
        .apply(
            DockCommand::MoveFloating {
                group: floating_id,
                origin: Point::new(100.0, 200.0),
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(outcome.new_version, version);
    assert_eq!(layout.version(), version);
    assert_eq!(
        layout.floating_group(floating_id).unwrap().bounds.origin,
        Point::new(100.0, 200.0)
    );
}

#[test]
fn panel_self_and_same_tab_position_resolve_to_noop() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, p0) = builder.panel(key("same-panel-a"), constraints());
    let (_, p1) = builder.panel(key("same-panel-b"), constraints());
    let group = builder.tabs(vec![p0, p1], 1, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let version = layout.version();

    for index in [0, 1] {
        let request = DragDropRequest {
            payload: DragPayload::Panel { panel },
            handle: DragHandle::PanelTab,
            origin: Point::ZERO,
            point: Point::new(4.0, 0.0),
            target: Some(DropTarget::Tabs { group, index }),
            floating_bounds: None,
        };
        assert_eq!(
            layout.resolve_drag_drop(&request, &metrics).unwrap(),
            DragDropAction::NoOp
        );
    }
    assert_eq!(layout.version(), version);
}

#[test]
fn panel_tab_drop_on_its_own_group_edge_is_no_op() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, p0) = builder.panel(key("own-edge-panel"), constraints());
    let (_, p1) = builder.panel(key("own-edge-sibling"), constraints());
    let group = builder.tabs(vec![p0, p1], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let request = DragDropRequest {
        payload: DragPayload::Panel { panel },
        handle: DragHandle::PanelTab,
        origin: Point::ZERO,
        point: Point::new(4.0, 0.0),
        target: Some(DropTarget::GroupEdge {
            group,
            zone: crate::DropZone::Bottom,
        }),
        floating_bounds: None,
    };

    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        DragDropAction::NoOp
    );
}

#[test]
fn panel_and_group_free_area_drops_require_adapter_bounds_and_resolve_to_float() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (panel, p0) = builder.panel(key("free-area-panel"), constraints());
    let (_, p1) = builder.panel(key("free-area-sibling"), constraints());
    let group = builder.tabs(vec![p0, p1], 1, true);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let floating_bounds = bounds(500.0, 120.0);

    let panel_request = DragDropRequest {
        payload: DragPayload::Panel { panel },
        handle: DragHandle::PanelTab,
        origin: Point::ZERO,
        point: Point::new(8.0, 0.0),
        target: Some(DropTarget::FreeArea),
        floating_bounds: Some(floating_bounds),
    };
    assert_eq!(
        layout.resolve_drag_drop(&panel_request, &metrics).unwrap(),
        DragDropAction::Command(DockCommand::Float {
            node: p0,
            bounds: floating_bounds,
        })
    );

    let group_request = DragDropRequest {
        payload: DragPayload::Group { group },
        handle: DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: Point::new(8.0, 0.0),
        target: Some(DropTarget::FreeArea),
        floating_bounds: Some(floating_bounds),
    };
    assert_eq!(
        layout.resolve_drag_drop(&group_request, &metrics).unwrap(),
        DragDropAction::Command(DockCommand::Float {
            node: group,
            bounds: floating_bounds,
        })
    );
    assert_eq!(layout.version(), 0);
}

#[test]
fn group_self_drop_and_same_column_slot_are_noops_without_version_increment() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("self-group-a"), constraints());
    let group = builder.tabs(vec![p0], 0, true);
    let (_, p1) = builder.panel(key("self-group-sibling"), constraints());
    let sibling = builder.tabs(vec![p1], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group, sibling],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();
    let version = layout.version();

    let self_request = DragDropRequest {
        payload: DragPayload::Group { group },
        handle: DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: Point::new(4.0, 0.0),
        target: Some(DropTarget::GroupTabs { group, index: 0 }),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&self_request, &metrics).unwrap(),
        DragDropAction::NoOp
    );

    for index in [0, 1] {
        let slot_request = DragDropRequest {
            target: Some(DropTarget::ColumnSlot { column, index }),
            floating_bounds: None,
            ..self_request
        };
        assert_eq!(
            layout.resolve_drag_drop(&slot_request, &metrics).unwrap(),
            DragDropAction::NoOp
        );
    }

    let outcome = layout
        .apply(
            DockCommand::MoveGroup {
                group,
                column,
                index: 0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(outcome.new_version, version);
    assert_eq!(layout.version(), version);
    assert!(tabs(&layout, group).collapsed);
}

#[test]
fn group_drag_to_main_column_becomes_one_command_and_preserves_tabs_state() {
    let metrics = DockMetrics::default();
    let (mut layout, source, _, target_column) = group_layout();
    let original = tabs(&layout, source).clone();
    let request = DragDropRequest {
        payload: DragPayload::Group { group: source },
        handle: DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: Point::new(12.0, 8.0),
        target: Some(DropTarget::MainColumn {
            side: DockSide::Right,
            index: 0,
        }),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveGroupToMainColumn {
            group: source,
            side: DockSide::Right,
            index: 0,
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("main column action");
    };
    layout.apply(command, &metrics).unwrap();
    assert_eq!(tabs(&layout, source), &original);
    assert!(layout.main_area(DockSide::Left).is_none());
    let Some(Node::Split(split)) = layout
        .main_area(DockSide::Right)
        .and_then(|area| layout.node(area.root))
    else {
        panic!("main-dock insertion produces a horizontal split");
    };
    assert_eq!(split.axis, Axis::X);
    let Node::Column(new_column) = layout.node(split.children[0]).unwrap() else {
        panic!("first inserted child is a column");
    };
    assert_eq!(new_column.groups, vec![source]);
    assert!(layout.node(target_column).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn sole_group_column_same_position_is_a_noop_command() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) = builder.panel(key("sole-column-panel"), constraints());
    let group = builder.tabs(vec![panel_node], 0, true);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();
    let version = layout.version();

    let request = DragDropRequest {
        payload: DragPayload::Group { group },
        handle: DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: Point::new(4.0, 0.0),
        target: Some(DropTarget::MainColumn {
            side: DockSide::Left,
            index: 0,
        }),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        DragDropAction::NoOp
    );

    let outcome = layout
        .apply(
            DockCommand::MoveGroupToMainColumn {
                group,
                side: DockSide::Left,
                index: 0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(outcome.new_version, version);
    assert_eq!(layout.version(), version);
    assert!(tabs(&layout, group).collapsed);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_capture_resolves_to_dock_floating_but_self_drop_is_noop() {
    let metrics = DockMetrics::default();
    let (layout, floating_id, floating_root_group) = floating_layout();
    let before = layout.version();
    let self_request = DragDropRequest {
        payload: DragPayload::FloatingGroup { group: floating_id },
        handle: DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(4.0, 0.0),
        target: Some(DropTarget::GroupTabs {
            group: floating_root_group,
            index: 1,
        }),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&self_request, &metrics).unwrap(),
        DragDropAction::NoOp
    );
    assert_eq!(layout.version(), before);

    // Build a fresh layout with a main target and a floating source so all IDs
    // are from the same arena.
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("capture-source-a"), constraints());
    let (_, p1) = builder.panel(key("capture-source-b"), constraints());
    let floating_tabs = builder.tabs(vec![p0, p1], 1, true);
    let original_floating_tabs = [p0, p1];
    let floating = builder.floating_group(floating_tabs, bounds(100.0, 200.0), 1);
    let (_, p2) = builder.panel(key("capture-target"), constraints());
    let main_tabs = builder.tabs(vec![p2], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![main_tabs],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let request = DragDropRequest {
        payload: DragPayload::FloatingGroup { group: floating },
        handle: DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(4.0, 0.0),
        target: Some(DropTarget::GroupTabs {
            group: main_tabs,
            index: 1,
        }),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        DragDropAction::Command(DockCommand::DockFloating {
            group: floating,
            target: DropTarget::GroupTabs {
                group: main_tabs,
                index: 1
            },
        })
    );
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    let DragDropAction::Command(command) = action else {
        panic!("floating capture docks");
    };
    layout.apply(command, &metrics).unwrap();
    assert_eq!(layout.floating_groups().count(), 0);
    assert_tabs_state(&layout, main_tabs, 2, false);
    let Node::Tabs(merged) = layout.node(main_tabs).unwrap() else {
        panic!("target remains Tabs");
    };
    assert_eq!(
        merged.children,
        vec![p2, original_floating_tabs[0], original_floating_tabs[1]]
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

fn assert_tabs_state(layout: &Layout, group: NodeId, active_index: usize, collapsed: bool) {
    let Node::Tabs(tabs) = layout.node(group).unwrap() else {
        panic!("expected Tabs");
    };
    assert_eq!(tabs.active_index, active_index);
    assert_eq!(tabs.collapsed, collapsed);
}

#[test]
fn rejected_drag_drop_request_does_not_mutate_or_invalidate_layout() {
    let metrics = DockMetrics::default();
    let (layout, source, target, _) = group_layout();
    let before = format!("{layout:?}");
    let version = layout.version();
    let request = DragDropRequest {
        payload: DragPayload::Group { group: source },
        handle: DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: Point::new(12.0, 8.0),
        target: Some(DropTarget::Tabs {
            group: target,
            index: 0,
        }),
        floating_bounds: None,
    };

    assert!(layout.resolve_drag_drop(&request, &metrics).is_err());
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), version);
    assert_eq!(layout.validate(&metrics), Ok(()));
}


#[test]
fn floating_capture_released_without_a_target_keeps_the_window_floating() {
    let metrics = DockMetrics::default();
    let (layout, floating_id, _) = floating_layout();
    let original = layout.floating_group(floating_id).unwrap().bounds;
    let version = layout.version();
    let payload = DragPayload::FloatingGroup { group: floating_id };
    let mut session = DragSession::new(&metrics).unwrap();

    session
        .pointer_down_with_handle(
            payload,
            DragHandle::FloatingCapture,
            Point::new(10.0, 20.0),
        )
        .unwrap();
    session.pointer_move(Point::new(15.0, 20.0)).unwrap();

    let request = session
        .pointer_up(Point::new(18.0, 24.0), None)
        .unwrap()
        .expect("an empty FloatingCapture release completes the gesture");

    assert_eq!(request.target, None);
    assert!(matches!(
        session.state(),
        crate::DragState::Dropped { target: None, .. }
    ));
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        DragDropAction::NoOp
    );
    assert_eq!(layout.version(), version);
    assert_eq!(layout.floating_group(floating_id).unwrap().bounds, original);
    assert_eq!(layout.floating_groups().count(), 1);
}

#[test]
fn floating_group_can_merge_into_another_floating_window() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_a) = builder.panel(key("floating-merge-source-a"), constraints());
    let (_, source_b) = builder.panel(key("floating-merge-source-b"), constraints());
    let source_root = builder.tabs(vec![source_a, source_b], 1, true);
    let source_float = builder.floating_group(source_root, bounds(40.0, 50.0), 1);

    let (_, target_panel) = builder.panel(key("floating-merge-target"), constraints());
    let target_root = builder.tabs(vec![target_panel], 0, false);
    let target_float = builder.floating_group(target_root, bounds(520.0, 80.0), 2);

    let mut layout = builder.build();
    let panel_count = layout.panel_instances().count();
    let request = DragDropRequest {
        payload: DragPayload::FloatingGroup { group: source_float },
        handle: DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(12.0, 8.0),
        target: Some(DropTarget::GroupTabs {
            group: target_root,
            index: 1,
        }),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::DockFloating {
            group: source_float,
            target: DropTarget::GroupTabs {
                group: target_root,
                index: 1,
            },
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("dropping onto the center of another floating group must merge tabs");
    };
    layout.apply(command, &metrics).unwrap();

    assert!(layout.floating_group(source_float).is_none());
    assert!(layout.floating_group(target_float).is_some());
    assert_tabs_state(&layout, target_root, 2, false);
    let Node::Tabs(merged) = layout.node(target_root).unwrap() else {
        panic!("the destination remains a Tabs group");
    };
    assert_eq!(merged.children, vec![target_panel, source_a, source_b]);
    assert_eq!(layout.panel_instances().count(), panel_count);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_group_can_split_against_another_floating_window_edge() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(key("floating-edge-source"), constraints());
    let source_root = builder.tabs(vec![source_panel], 0, false);
    let source_float = builder.floating_group(source_root, bounds(40.0, 50.0), 1);

    let (_, target_panel) = builder.panel(key("floating-edge-target"), constraints());
    let target_root = builder.tabs(vec![target_panel], 0, false);
    let target_float = builder.floating_group(target_root, bounds(520.0, 80.0), 2);

    let mut layout = builder.build();
    let request = DragDropRequest {
        payload: DragPayload::FloatingGroup { group: source_float },
        handle: DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(12.0, 8.0),
        target: Some(DropTarget::GroupEdge {
            group: target_root,
            zone: crate::DropZone::Bottom,
        }),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::DockFloating {
            group: source_float,
            target: DropTarget::GroupEdge {
                group: target_root,
                zone: crate::DropZone::Bottom,
            },
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("dropping on a floating stack edge must create a split");
    };
    layout.apply(command, &metrics).unwrap();

    assert!(layout.floating_group(source_float).is_none());
    assert!(layout.floating_group(target_float).is_some());
    let target_area = layout.floating_group(target_float).unwrap().area;
    let root = layout.dock_area(target_area).unwrap().root;
    let Some(Node::Split(split)) = layout.node(root) else {
        panic!("the destination floating window now owns a vertical split");
    };
    assert_eq!(split.axis, Axis::Y);
    assert_eq!(split.children, vec![target_root, source_root]);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn extracting_one_panel_from_a_floating_group_returns_its_new_window_id() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (moving_panel, moving_node) =
        builder.panel(key("floating-extract-moving"), constraints());
    let (remaining_panel, remaining_node) =
        builder.panel(key("floating-extract-remaining"), constraints());
    let source_root = builder.tabs(vec![moving_node, remaining_node], 0, false);
    let source_float = builder.floating_group(source_root, bounds(40.0, 50.0), 1);
    let mut layout = builder.build();

    let request = DragDropRequest {
        payload: DragPayload::Panel { panel: moving_panel },
        handle: DragHandle::PanelTab,
        origin: Point::ZERO,
        point: Point::new(12.0, 8.0),
        target: Some(DropTarget::FreeArea),
        floating_bounds: Some(bounds(520.0, 80.0)),
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    let DragDropAction::Command(command) = action else {
        panic!("dropping a panel into free space must create a floating group");
    };
    let outcome = layout.apply(command, &metrics).unwrap();
    let new_float = match outcome.actual_value {
        Some(crate::CommandValue::FloatingGroup(group)) => group,
        other => panic!("Float must return the created FloatingGroupId, got {other:?}"),
    };

    assert_ne!(new_float, source_float);
    assert!(layout.floating_group(source_float).is_some());
    assert!(layout.floating_group(new_float).is_some());
    assert!(matches!(
        layout.node(source_root),
        Some(Node::Tabs(tabs)) if tabs.children == vec![remaining_node]
    ));
    let new_area = layout.floating_group(new_float).unwrap().area;
    let new_root = layout.dock_area(new_area).unwrap().root;
    let Some(Node::Column(column)) = layout.node(new_root) else {
        panic!("a detached Photoshop panel must use a floating Column root");
    };
    assert_eq!(column.groups.len(), 1);
    assert!(matches!(
        layout.node(column.groups[0]),
        Some(Node::Tabs(tabs)) if tabs.children == vec![moving_node]
    ));
    assert!(layout.panel_instance(moving_panel).is_some());
    assert!(layout.panel_instance(remaining_panel).is_some());
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.floating_groups().count(), 2);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn photoshop_main_dock_area_can_float_all_columns_as_one_composition() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, left_panel) = builder.panel(key("dock-area-left-panel"), constraints());
    let left_group = builder.tabs(vec![left_panel], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![left_group],
        vec![1.0],
    );

    let (_, right_panel) = builder.panel(key("dock-area-right-panel"), constraints());
    let right_group = builder.tabs(vec![right_panel], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![right_group],
        vec![1.0],
    );

    let root = builder.split(Axis::X, vec![left_column, right_column], vec![0.5, 0.5]);
    builder.main_area(DockSide::Right, root);
    let mut layout = builder.build();

    let outcome = layout
        .apply(
            DockCommand::Float {
                node: root,
                bounds: Rect::new(Point::new(40.0, 50.0), Size::new(720.0, 420.0)),
            },
            &metrics,
        )
        .unwrap();
    let floating = match outcome.actual_value {
        Some(crate::CommandValue::FloatingGroup(group)) => group,
        other => panic!("floating the DockArea must return its FloatingGroupId, got {other:?}"),
    };

    assert!(layout.main_area(DockSide::Right).is_none());
    let floating_area = layout
        .dock_area(layout.floating_group(floating).unwrap().area)
        .unwrap();
    assert_eq!(floating_area.root, root);
    let Some(Node::Split(split)) = layout.node(floating_area.root) else {
        panic!("the floating DockArea must preserve its horizontal Split(X) root");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children, vec![left_column, right_column]);
    assert!(matches!(layout.node(left_column), Some(Node::Column(_))));
    assert!(matches!(layout.node(right_column), Some(Node::Column(_))));
    assert_eq!(layout.panel_instances().count(), 2);
    assert_eq!(layout.floating_groups().count(), 1);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_composition_with_split_root_can_build_drop_zones_and_dock() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);

    let (_, source_left_panel) = builder.panel(key("capture-split-left"), constraints());
    let source_left_group = builder.tabs(vec![source_left_panel], 0, false);

    let (_, source_right_panel) = builder.panel(key("capture-split-right"), constraints());
    let source_right_group = builder.tabs(vec![source_right_panel], 0, false);

    // Generic floating roots can be a horizontal Split of tabs directly.
    // Docking this tree into Main Dock must wrap each branch in a Column.
    let source_root = builder.split(
        Axis::X,
        vec![source_left_group, source_right_group],
        vec![0.5, 0.5],
    );
    let source_floating = builder.floating_group(source_root, bounds(80.0, 90.0), 1);

    let (_, target_panel) = builder.panel(key("capture-split-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let layout = builder.build();
    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            &metrics,
        )
        .unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Group { group: source_root }, &rects, &metrics)
        .unwrap();
    let target = zones
        .iter()
        .find_map(|zone| match zone.target {
            DropTarget::MainColumn {
                side: DockSide::Right,
                index: 1,
            } if zone.zone == crate::DropZone::DockEdge => Some(zone.target),
            _ => None,
        })
        .expect("the main Dock edge must accept the complete floating tree");

    let request = crate::DragDropRequest {
        payload: crate::DragPayload::FloatingGroup {
            group: source_floating,
        },
        handle: crate::DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(1198.0, 400.0),
        target: Some(target),
        floating_bounds: None,
    };
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    let crate::DragDropAction::Command(command) = action else {
        panic!("docking a complete multi-column floating composition must create a command");
    };
    assert_eq!(
        command,
        DockCommand::DockFloating {
            group: source_floating,
            target,
        }
    );

    let mut layout = layout;
    layout.apply(command, &metrics).unwrap();
    assert_eq!(layout.floating_groups().count(), 0);
    let right_root = layout
        .main_area(DockSide::Right)
        .expect("the right main dock must remain present")
        .root;
    let Some(Node::Split(split)) = layout.node(right_root) else {
        panic!("the right main dock should contain the merged horizontal composition");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 3);
    assert!(
        split
            .children
            .iter()
            .all(|child| matches!(layout.node(*child), Some(Node::Column(_))))
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_group_can_dock_to_an_empty_main_window_edge() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, panel_node) = builder.panel(key("empty-main-edge-panel"), constraints());
    let source_root = builder.tabs(vec![panel_node], 0, false);
    let floating_group = builder.floating_group(source_root, bounds(100.0, 120.0), 1);
    let mut layout = builder.build();

    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            &metrics,
        )
        .unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Group { group: source_root }, &rects, &metrics)
        .unwrap();
    let left_edge = zones
        .iter()
        .find(|zone| {
            zone.zone == crate::DropZone::DockEdge
                && zone.target
                    == (DropTarget::MainColumn {
                        side: DockSide::Left,
                        index: 0,
                    })
        })
        .expect("an empty left Main Dock must expose an outer-edge insertion target");

    let request = crate::DragDropRequest {
        payload: crate::DragPayload::FloatingGroup {
            group: floating_group,
        },
        handle: crate::DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(2.0, 400.0),
        target: Some(left_edge.target),
        floating_bounds: None,
    };
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    let crate::DragDropAction::Command(command) = action else {
        panic!("docking a floating panel into an empty main side must create a command");
    };
    layout.apply(command, &metrics).unwrap();
    assert_eq!(layout.floating_groups().count(), 0);
    assert!(layout.main_area(DockSide::Left).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn split_root_floating_composition_can_create_first_main_dock_columns() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, left_panel) = builder.panel(key("empty-split-left"), constraints());
    let left_group = builder.tabs(vec![left_panel], 0, false);
    let (_, right_panel) = builder.panel(key("empty-split-right"), constraints());
    let right_group = builder.tabs(vec![right_panel], 0, false);
    let source_root = builder.split(Axis::X, vec![left_group, right_group], vec![0.4, 0.6]);
    let floating_group = builder.floating_group(source_root, bounds(120.0, 90.0), 1);
    let mut layout = builder.build();

    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            &metrics,
        )
        .unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Group { group: source_root }, &rects, &metrics)
        .unwrap();
    let target = zones
        .iter()
        .find(|zone| {
            zone.zone == crate::DropZone::DockEdge
                && zone.target
                    == (DropTarget::MainColumn {
                        side: DockSide::Left,
                        index: 0,
                    })
        })
        .expect("empty left Main Dock must accept the whole horizontal composition")
        .target;
    let request = crate::DragDropRequest {
        payload: crate::DragPayload::FloatingGroup {
            group: floating_group,
        },
        handle: crate::DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(2.0, 400.0),
        target: Some(target),
        floating_bounds: None,
    };
    let crate::DragDropAction::Command(command) =
        layout.resolve_drag_drop(&request, &metrics).unwrap()
    else {
        panic!("the empty main edge must resolve to DockFloating");
    };

    layout.apply(command, &metrics).unwrap();
    assert_eq!(layout.floating_groups().count(), 0);
    let root = layout.main_area(DockSide::Left).unwrap().root;
    let Some(Node::Split(split)) = layout.node(root) else {
        panic!("the horizontal composition root should remain a Split");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 2);
    assert!(split.children.iter().all(|child| {
        matches!(layout.node(*child), Some(Node::Column(_)))
    }));
    assert_eq!(layout.validate(&metrics), Ok(()));
}


#[test]
fn column_header_drag_moves_a_complete_column_between_main_docks() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_panel) = builder.panel(key("column-drag-first"), constraints());
    let first_group = builder.tabs(vec![first_panel], 0, false);
    let (_, second_panel) = builder.panel(key("column-drag-second"), constraints());
    let second_group = builder.tabs(vec![second_panel], 0, true);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group],
        vec![0.4, 0.6],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(key("column-drag-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            &metrics,
        )
        .unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Column { column: source_column }, &rects, &metrics)
        .unwrap();

    assert!(zones.iter().all(|zone| matches!(
        zone.target,
        DropTarget::MainColumn { .. } | DropTarget::FreeArea
    )));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::MainColumn {
                side: DockSide::Right,
                index: 0,
            }
    }));
    assert!(zones.iter().any(|zone| zone.target == DropTarget::FreeArea));

    let floating_bounds = bounds(520.0, 140.0);
    let free_area_request = DragDropRequest {
        payload: DragPayload::Column {
            column: source_column,
        },
        handle: DragHandle::ColumnHeader,
        origin: Point::ZERO,
        point: Point::new(600.0, 400.0),
        target: Some(DropTarget::FreeArea),
        floating_bounds: Some(floating_bounds),
    };
    assert_eq!(
        layout.resolve_drag_drop(&free_area_request, &metrics).unwrap(),
        DragDropAction::Command(DockCommand::Float {
            node: source_column,
            bounds: floating_bounds,
        })
    );

    let request = DragDropRequest {
        target: Some(DropTarget::MainColumn {
            side: DockSide::Right,
            index: 0,
        }),
        floating_bounds: None,
        ..free_area_request
    };
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveColumnToTarget {
            column: source_column,
            target: DropTarget::MainColumn {
                side: DockSide::Right,
                index: 0,
            },
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("dropping on a Main Dock column edge must move the whole Column");
    };
    let outcome = layout.apply(command, &metrics).unwrap();

    assert!(layout.main_area(DockSide::Left).is_none());
    assert_eq!(
        layout.main_columns(DockSide::Right).unwrap(),
        vec![source_column, target_column]
    );
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(column))
            if column.groups == vec![first_group, second_group]
    ));
    assert_eq!(
        outcome.inverse,
        Some(DockCommand::MoveColumnToMainColumn {
            column: source_column,
            side: DockSide::Left,
            index: 0,
        })
    );

    layout.apply(outcome.inverse.unwrap(), &metrics).unwrap();
    assert_eq!(
        layout.main_columns(DockSide::Left).unwrap(),
        vec![source_column]
    );
    assert_eq!(
        layout.main_columns(DockSide::Right).unwrap(),
        vec![target_column]
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn column_header_can_reorder_within_a_main_dock_at_max_column_capacity() {
    let metrics = DockMetrics {
        max_columns: 2,
        ..DockMetrics::default()
    };
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_panel) = builder.panel(key("column-capacity-first"), constraints());
    let first_group = builder.tabs(vec![first_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group],
        vec![1.0],
    );

    let (_, second_panel) = builder.panel(key("column-capacity-second"), constraints());
    let second_group = builder.tabs(vec![second_panel], 0, false);
    let second_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![second_group],
        vec![1.0],
    );

    let root = builder.split(
        Axis::X,
        vec![source_column, second_column],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, root);

    let layout = builder.build();
    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            &metrics,
        )
        .unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Column { column: source_column }, &rects, &metrics)
        .unwrap();

    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::MainColumn {
                side: DockSide::Left,
                index: 2,
            }
            && zone.zone == crate::DropZone::ColumnRight
    }));
}

#[test]
fn column_header_drag_reorders_columns_within_one_main_dock() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(key("column-reorder-source"), constraints());
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );

    let (_, target_panel) = builder.panel(key("column-reorder-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    let root = builder.split(
        Axis::X,
        vec![source_column, target_column],
        vec![0.35, 0.65],
    );
    builder.main_area(DockSide::Left, root);

    let mut layout = builder.build();
    let request = DragDropRequest {
        payload: DragPayload::Column {
            column: source_column,
        },
        handle: DragHandle::ColumnHeader,
        origin: Point::ZERO,
        point: Point::new(20.0, 20.0),
        target: Some(DropTarget::MainColumn {
            side: DockSide::Left,
            index: 2,
        }),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveColumnToTarget {
            column: source_column,
            target: DropTarget::MainColumn {
                side: DockSide::Left,
                index: 2,
            },
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("moving to the final insertion slot must reorder the Column");
    };
    let outcome = layout.apply(command, &metrics).unwrap();

    assert_eq!(
        layout.main_columns(DockSide::Left).unwrap(),
        vec![target_column, source_column]
    );
    assert_eq!(
        outcome.inverse,
        Some(DockCommand::MoveColumnToMainColumn {
            column: source_column,
            side: DockSide::Left,
            index: 0,
        })
    );

    layout.apply(outcome.inverse.unwrap(), &metrics).unwrap();
    assert_eq!(
        layout.main_columns(DockSide::Left).unwrap(),
        vec![source_column, target_column]
    );
    let Some(Node::Split(split)) = layout.node(root) else {
        panic!("the original Split(X) root remains the main dock composition");
    };
    assert_eq!(split.ratios, vec![0.35, 0.65]);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn photoshop_column_can_float_as_one_composition_and_dock_back() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_panel) = builder.panel(key("column-float-first"), constraints());
    let first_group = builder.tabs(vec![first_panel], 0, false);
    let (_, second_panel) = builder.panel(key("column-float-second"), constraints());
    let second_group = builder.tabs(vec![second_panel], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group],
        vec![0.4, 0.6],
    );
    builder.main_area(DockSide::Left, column);

    let mut layout = builder.build();
    assert_eq!(layout.validate(&metrics), Ok(()));

    let outcome = layout
        .apply(
            DockCommand::Float {
                node: column,
                bounds: bounds(480.0, 120.0),
            },
            &metrics,
        )
        .unwrap();
    let Some(crate::CommandValue::FloatingGroup(floating_group)) = outcome.actual_value else {
        panic!("floating a Column must return its new FloatingGroupId");
    };

    assert!(layout.main_area(DockSide::Left).is_none());
    let floating_area = layout.floating_group(floating_group).unwrap().area;
    assert_eq!(layout.dock_area(floating_area).unwrap().root, column);
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(value))
            if value.groups == vec![first_group, second_group]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));

    layout
        .apply(
            DockCommand::DockFloating {
                group: floating_group,
                target: DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                },
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(floating_group).is_none());
    assert_eq!(layout.main_area(DockSide::Left).unwrap().root, column);
    assert!(matches!(
        layout.node(column),
        Some(Node::Column(value))
            if value.groups == vec![first_group, second_group]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}
#[test]
fn floating_column_capture_uses_column_drop_zones_and_docks_as_one_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_panel) = builder.panel(key("floating-column-first"), constraints());
    let first_group = builder.tabs(vec![first_panel], 0, false);
    let (_, second_panel) = builder.panel(key("floating-column-second"), constraints());
    let second_group = builder.tabs(vec![second_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group],
        vec![0.35, 0.65],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(key("floating-column-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    let float_outcome = layout
        .apply(
            DockCommand::Float {
                node: source_column,
                bounds: bounds(520.0, 140.0),
            },
            &metrics,
        )
        .unwrap();
    let Some(crate::CommandValue::FloatingGroup(floating_group)) = float_outcome.actual_value
    else {
        panic!("floating the Column must create a FloatingGroup");
    };

    let rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            &metrics,
        )
        .unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Column { column: source_column }, &rects, &metrics)
        .unwrap();

    assert!(zones.iter().all(|zone| matches!(
        zone.target,
        DropTarget::MainColumn { .. } | DropTarget::FreeArea
    )));
    let target = DropTarget::MainColumn {
        side: DockSide::Right,
        index: 0,
    };
    assert!(zones.iter().any(|zone| zone.target == target));

    let request = DragDropRequest {
        payload: DragPayload::FloatingGroup {
            group: floating_group,
        },
        handle: DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(1000.0, 300.0),
        target: Some(target),
        floating_bounds: None,
    };
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::DockFloating {
            group: floating_group,
            target,
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("the floating Column should dock as one composition");
    };
    layout.apply(command, &metrics).unwrap();

    assert!(layout.floating_group(floating_group).is_none());
    assert_eq!(
        layout.main_columns(DockSide::Right).unwrap(),
        vec![source_column, target_column]
    );
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(value))
            if value.groups == vec![first_group, second_group]
                && value.group_ratios == vec![0.35, 0.65]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn photoshop_main_column_moves_directly_into_an_existing_floating_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_first_panel) =
        builder.panel(key("main-column-to-floating-first"), constraints());
    let source_first_group = builder.tabs(vec![source_first_panel], 0, false);
    let (_, source_second_panel) =
        builder.panel(key("main-column-to-floating-second"), constraints());
    let source_second_group = builder.tabs(vec![source_second_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_first_group, source_second_group],
        vec![0.35, 0.65],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_first_panel) =
        builder.panel(key("floating-column-target-first"), constraints());
    let target_first_group = builder.tabs(vec![target_first_panel], 0, false);
    let (_, target_second_panel) =
        builder.panel(key("floating-column-target-second"), constraints());
    let target_second_group = builder.tabs(vec![target_second_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_first_group, target_second_group],
        vec![0.4, 0.6],
    );
    let target_floating = builder.floating_group(target_column, bounds(620.0, 140.0), 1);
    let mut layout = builder.build();

    let target_rects = layout
        .compute_floating_rects(target_floating, &metrics)
        .unwrap();
    let source = DropSource::Column {
        column: source_column,
    };
    let zones = layout
        .compute_drop_zones(source, &target_rects, &metrics)
        .unwrap();
    let left_zone = zones
        .iter()
        .find(|zone| {
            zone.target
                == DropTarget::GroupEdge {
                    group: target_first_group,
                    zone: crate::DropZone::Left,
                }
                && zone.zone == crate::DropZone::Left
        })
        .expect("a Floating Column must expose its left edge to a Main Column");
    let point = Point::new(
        left_zone.rect.origin.x + left_zone.rect.size.width / 2.0,
        left_zone.rect.origin.y + left_zone.rect.size.height / 2.0,
    );
    let target = DropTarget::GroupEdge {
        group: target_first_group,
        zone: crate::DropZone::Left,
    };

    assert_eq!(
        layout
            .resolve_drop(point, source, &target_rects, &metrics)
            .unwrap(),
        Some(target)
    );

    let request = DragDropRequest {
        payload: DragPayload::Column {
            column: source_column,
        },
        handle: DragHandle::ColumnHeader,
        origin: Point::ZERO,
        point,
        target: Some(target),
        floating_bounds: None,
    };
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveColumnToTarget {
            column: source_column,
            target,
        })
    );

    let DragDropAction::Command(command) = action else {
        panic!("the Main Column must move directly into the Floating composition");
    };
    layout.apply(command, &metrics).unwrap();

    assert!(layout.main_area(DockSide::Left).is_none());
    let target_area = layout.floating_group(target_floating).unwrap().area;
    let target_root = layout.dock_area(target_area).unwrap().root;
    let Some(Node::Split(split)) = layout.node(target_root) else {
        panic!("joining a Main Column must create a horizontal Floating composition");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children, vec![source_column, target_column]);
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(column))
            if column.groups == vec![source_first_group, source_second_group]
                && column.group_ratios == vec![0.35, 0.65]
    ));
    assert!(matches!(
        layout.node(target_column),
        Some(Node::Column(column))
            if column.groups == vec![target_first_group, target_second_group]
                && column.group_ratios == vec![0.4, 0.6]
    ));
    assert_eq!(layout.panel_instances().count(), 4);
    assert_eq!(layout.floating_groups().count(), 1);
    assert_eq!(layout.validate(&metrics), Ok(()));
}


#[test]
fn floating_column_header_moves_directly_to_main_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(key("floating-column-to-main"), constraints());
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    let (_, sibling_panel) = builder.panel(key("floating-column-sibling"), constraints());
    let sibling_group = builder.tabs(vec![sibling_panel], 0, false);
    let sibling_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![sibling_group],
        vec![1.0],
    );
    let source_root = builder.split(
        Axis::X,
        vec![source_column, sibling_column],
        vec![0.4, 0.6],
    );
    let source_floating = builder.floating_group(source_root, bounds(100.0, 140.0), 1);

    let (_, target_panel) = builder.panel(key("floating-column-to-main-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    let target = DropTarget::MainColumn {
        side: DockSide::Right,
        index: 0,
    };
    let request = DragDropRequest {
        payload: DragPayload::Column {
            column: source_column,
        },
        handle: DragHandle::ColumnHeader,
        origin: Point::ZERO,
        point: Point::new(600.0, 300.0),
        target: Some(target),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveColumnToTarget {
            column: source_column,
            target,
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("a Floating Column dropped on Main must produce one direct move command");
    };
    layout.apply(command, &metrics).unwrap();

    let source_area = layout.floating_group(source_floating).unwrap().area;
    assert_eq!(layout.dock_area(source_area).unwrap().root, sibling_column);
    assert_eq!(
        layout.main_columns(DockSide::Right).unwrap(),
        vec![source_column, target_column]
    );
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(column)) if column.groups == vec![source_group]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_column_moves_directly_into_another_floating_composition() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(key("floating-to-floating-source"), constraints());
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    let (_, sibling_panel) = builder.panel(key("floating-column-sibling"), constraints());
    let sibling_group = builder.tabs(vec![sibling_panel], 0, false);
    let sibling_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![sibling_group],
        vec![1.0],
    );
    let source_root = builder.split(
        Axis::X,
        vec![source_column, sibling_column],
        vec![0.4, 0.6],
    );
    let source_floating = builder.floating_group(source_root, bounds(100.0, 140.0), 1);

    let (_, target_panel) = builder.panel(key("floating-to-floating-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    let target_floating = builder.floating_group(target_column, bounds(620.0, 140.0), 2);

    let mut layout = builder.build();
    let target = DropTarget::GroupEdge {
        group: target_group,
        zone: crate::DropZone::Right,
    };
    let request = DragDropRequest {
        payload: DragPayload::Column {
            column: source_column,
        },
        handle: DragHandle::ColumnHeader,
        origin: Point::ZERO,
        point: Point::new(900.0, 300.0),
        target: Some(target),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MoveColumnToTarget {
            column: source_column,
            target,
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("moving one Floating Column must not dock the entire source window");
    };
    layout.apply(command, &metrics).unwrap();

    let source_area = layout.floating_group(source_floating).unwrap().area;
    assert_eq!(layout.dock_area(source_area).unwrap().root, sibling_column);
    let target_area = layout.floating_group(target_floating).unwrap().area;
    let root = layout.dock_area(target_area).unwrap().root;
    let Some(Node::Split(split)) = layout.node(root) else {
        panic!("the destination Floating area must keep both Columns in one composition");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children, vec![target_column, source_column]);
    assert_eq!(layout.floating_groups().count(), 2);
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(column)) if column.groups == vec![source_group]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn main_group_can_split_into_the_outer_edge_of_a_floating_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_panel) = builder.panel(key("main-group-to-floating-a"), constraints());
    let (_, second_panel) = builder.panel(key("main-group-to-floating-b"), constraints());
    let source_group = builder.tabs(vec![first_panel, second_panel], 1, true);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(key("main-group-to-floating-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    let target_floating = builder.floating_group(target_column, bounds(620.0, 140.0), 1);
    let mut layout = builder.build();

    let target = DropTarget::GroupEdge {
        group: target_group,
        zone: crate::DropZone::Left,
    };
    let request = DragDropRequest {
        payload: DragPayload::Group {
            group: source_group,
        },
        handle: DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: Point::new(800.0, 300.0),
        target: Some(target),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::SplitGroup {
            group: source_group,
            target_group,
            zone: crate::DropZone::Left,
            ratio: 0.5,
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("a whole group dropped on a Floating Column edge must split it");
    };
    layout.apply(command, &metrics).unwrap();

    assert!(layout.main_area(DockSide::Left).is_none());
    let target_area = layout.floating_group(target_floating).unwrap().area;
    let root = layout.dock_area(target_area).unwrap().root;
    let Some(Node::Split(split)) = layout.node(root) else {
        panic!("the group must become a new Column beside the Floating target");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children[1], target_column);
    assert!(matches!(
        layout.node(split.children[0]),
        Some(Node::Column(column)) if column.groups == vec![source_group]
    ));
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs)) if tabs.children.len() == 2 && tabs.active_index == 1
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn main_panel_can_split_into_the_outer_edge_of_a_floating_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_panel_node) =
        builder.panel(key("main-panel-to-floating-source"), constraints());
    let (_, remaining_panel_node) =
        builder.panel(key("main-panel-to-floating-remaining"), constraints());
    let source_group = builder.tabs(vec![source_panel_node, remaining_panel_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(key("main-panel-to-floating-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    let target_floating = builder.floating_group(target_column, bounds(620.0, 140.0), 1);
    let mut layout = builder.build();

    let target = DropTarget::GroupEdge {
        group: target_group,
        zone: crate::DropZone::Right,
    };
    let request = DragDropRequest {
        payload: DragPayload::Panel { panel: source_panel },
        handle: DragHandle::PanelTab,
        origin: Point::ZERO,
        point: Point::new(900.0, 300.0),
        target: Some(target),
        floating_bounds: None,
    };

    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        DragDropAction::Command(DockCommand::MovePanel {
            panel: source_panel,
            target,
        })
    );
    let DragDropAction::Command(command) = action else {
        panic!("a dragged Panel must be moved as one panel into the Floating edge");
    };
    layout.apply(command, &metrics).unwrap();

    let target_area = layout.floating_group(target_floating).unwrap().area;
    let root = layout.dock_area(target_area).unwrap().root;
    let Some(Node::Split(split)) = layout.node(root) else {
        panic!("the detached Panel must be wrapped in a Column beside the Floating target");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children[0], target_column);
    let Some(Node::Column(new_column)) = layout.node(split.children[1]) else {
        panic!("the moved Panel must live in a new Column");
    };
    let [new_group] = new_column.groups.as_slice() else {
        panic!("the new Column must contain exactly the moved Panel group");
    };
    assert!(matches!(
        layout.node(*new_group),
        Some(Node::Tabs(tabs)) if tabs.children.len() == 1
            && matches!(layout.node(tabs.children[0]), Some(Node::Panel(panel)) if panel.panel_id == source_panel)
    ));
    assert!(matches!(
        layout.node(source_group),
        Some(Node::Tabs(tabs)) if tabs.children == vec![remaining_panel_node]
    ));
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn direct_column_move_rejects_invalid_target_atomically_and_undo_restores_source() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(key("column-atomic-source"), constraints());
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_panel) = builder.panel(key("column-atomic-target"), constraints());
    let target_group = builder.tabs(vec![target_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    let target_floating = builder.floating_group(target_column, bounds(620.0, 140.0), 1);
    let mut layout = builder.build();

    let before_invalid = format!("{layout:?}");
    assert!(
        layout
            .apply(
                DockCommand::MoveColumnToTarget {
                    column: source_column,
                    target: DropTarget::GroupTabs {
                        group: target_group,
                        index: 0,
                    },
                },
                &metrics,
            )
            .is_err()
    );
    assert_eq!(format!("{layout:?}"), before_invalid);

    let target = DropTarget::GroupEdge {
        group: target_group,
        zone: crate::DropZone::Left,
    };
    let mut history = crate::UndoRedoHistory::new();
    history
        .apply(
            &mut layout,
            DockCommand::MoveColumnToTarget {
                column: source_column,
                target,
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.main_area(DockSide::Left).is_none());
    assert!(layout.floating_group(target_floating).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));

    assert!(history.undo(&mut layout, &metrics).unwrap());
    assert_eq!(
        layout.main_area(DockSide::Left).unwrap().root,
        source_column
    );
    let target_area = layout.floating_group(target_floating).unwrap().area;
    assert_eq!(layout.dock_area(target_area).unwrap().root, target_column);
    assert!(matches!(
        layout.node(source_column),
        Some(Node::Column(column)) if column.groups == vec![source_group]
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}
