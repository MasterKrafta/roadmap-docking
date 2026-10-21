use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DragDropAction, DragDropRequest,
    DragHandle, DragPayload, DragSession, DragSessionError, DropTarget, FloatingGroupId, Layout, Node, NodeId, PanelConstraints, PanelId, PanelKey, Point, Profile,
    Rect, Size, Tabs,
};

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).unwrap()
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
    let request = session.pointer_up(Point::new(30.0, 25.0), None).unwrap().unwrap();

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

    let DragDropAction::Command(command) = action else { panic!("window moved"); };
    let outcome = layout.apply(command, &metrics).unwrap();
    assert_eq!(outcome.new_version, version + 1);
    assert_eq!(layout.floating_group(floating_id).unwrap().bounds.origin, Point::new(120.0, 205.0));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_title_released_at_its_original_point_is_a_noop_without_version_change() {
    let metrics = DockMetrics::default();
    let (mut layout, floating_id, _) = floating_layout();
    let version = layout.version();
    let payload = DragPayload::FloatingGroup { group: floating_id };
    let mut session = DragSession::new(&metrics).unwrap();

    session.pointer_down_with_handle(
        payload,
        DragHandle::FloatingTitle,
        Point::new(40.0, 50.0),
    ).unwrap();
    session.pointer_move(Point::new(44.0, 50.0)).unwrap();
    let request = session.pointer_up(Point::new(40.0, 50.0), None).unwrap().unwrap();

    assert_eq!(layout.resolve_drag_drop(&request, &metrics).unwrap(), DragDropAction::NoOp);
    assert_eq!(layout.version(), version);
    assert_eq!(layout.floating_group(floating_id).unwrap().bounds.origin, Point::new(100.0, 200.0));
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
        };
        assert_eq!(layout.resolve_drag_drop(&request, &metrics).unwrap(), DragDropAction::NoOp);
    }
    assert_eq!(layout.version(), version);
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
    };
    assert_eq!(layout.resolve_drag_drop(&self_request, &metrics).unwrap(), DragDropAction::NoOp);

    for index in [0, 1] {
        let slot_request = DragDropRequest {
            target: Some(DropTarget::ColumnSlot { column, index }),
            ..self_request
        };
        assert_eq!(layout.resolve_drag_drop(&slot_request, &metrics).unwrap(), DragDropAction::NoOp);
    }

    let outcome = layout.apply(DockCommand::MoveGroup { group, column, index: 0 }, &metrics).unwrap();
    assert_eq!(outcome.new_version, version);
    assert_eq!(layout.version(), version);
    assert_eq!(tabs(&layout, group).collapsed, true);
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
        target: Some(DropTarget::MainColumn { side: DockSide::Right, index: 0 }),
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
    let DragDropAction::Command(command) = action else { panic!("main column action"); };
    layout.apply(command, &metrics).unwrap();
    assert_eq!(tabs(&layout, source), &original);
    assert_eq!(layout.main_area(DockSide::Left).unwrap().root != NodeId::null(), true);
    let Some(Node::Split(split)) = layout.main_area(DockSide::Right).and_then(|area| layout.node(area.root)) else {
        panic!("main-dock insertion produces a horizontal split");
    };
    assert_eq!(split.axis, Axis::X);
    let Node::Column(new_column) = layout.node(split.children[0]).unwrap() else {
        panic!("first inserted child is a column");
    };
    assert_eq!(new_column.groups, vec![source]);
    assert_eq!(layout.node(target_column).is_some(), true);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_capture_resolves_to_dock_floating_but_self_drop_is_noop() {
    let metrics = DockMetrics::default();
    let (mut layout, floating_id, floating_root_group) = floating_layout();
    let before = layout.version();
    let self_request = DragDropRequest {
        payload: DragPayload::FloatingGroup { group: floating_id },
        handle: DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(4.0, 0.0),
        target: Some(DropTarget::GroupTabs { group: floating_root_group, index: 1 }),
    };
    assert_eq!(layout.resolve_drag_drop(&self_request, &metrics).unwrap(), DragDropAction::NoOp);
    assert_eq!(layout.version(), before);

    // Build a fresh layout with a main target and a floating source so all IDs
    // are from the same arena.
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, p0) = builder.panel(key("capture-source-a"), constraints());
    let (_, p1) = builder.panel(key("capture-source-b"), constraints());
    let floating_tabs = builder.tabs(vec![p0, p1], 1, true);
    let floating = builder.floating_group(floating_tabs, bounds(100.0, 200.0), 1);
    let (_, p2) = builder.panel(key("capture-target"), constraints());
    let main_tabs = builder.tabs(vec![p2], 0, false);
    let column = builder.column(metrics.default_column_width, ColumnMode::Expanded, vec![main_tabs], vec![1.0]);
    builder.main_area(DockSide::Right, column);
    let mut layout = builder.build();
    let request = DragDropRequest {
        payload: DragPayload::FloatingGroup { group: floating },
        handle: DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(4.0, 0.0),
        target: Some(DropTarget::GroupTabs { group: main_tabs, index: 1 }),
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        DragDropAction::Command(DockCommand::DockFloating {
            group: floating,
            target: DropTarget::GroupTabs { group: main_tabs, index: 1 },
        })
    );
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    let DragDropAction::Command(command) = action else { panic!("floating capture docks"); };
    layout.apply(command, &metrics).unwrap();
    assert_eq!(layout.floating_groups().count(), 0);
    assert_tabs_state(&layout, main_tabs, 2, false);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

fn assert_tabs_state(layout: &Layout, group: NodeId, active_index: usize, collapsed: bool) {
    let Node::Tabs(tabs) = layout.node(group).unwrap() else { panic!("expected Tabs"); };
    assert_eq!(tabs.active_index, active_index);
    assert_eq!(tabs.collapsed, collapsed);
}
