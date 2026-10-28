use slotmap::SlotMap;

use super::DragSession;
use crate::{
    DockAreaId, DockMetrics, DragCancelReason, DragDropRequest, DragHandle, DragPayload,
    DragPreview, DragSessionError, DragState, DropTarget, FloatingGroupId, Layout, Node, NodeId,
    PanelConstraints, PanelId, PanelKey, Point, Profile, Size, create_initial_layout,
};

fn panel_payload() -> DragPayload {
    let mut panels = SlotMap::<PanelId, ()>::with_key();
    DragPayload::Panel {
        panel: panels.insert(()),
    }
}

fn session() -> DragSession {
    DragSession::new(&DockMetrics::default()).expect("default drag metrics are valid")
}

fn test_layout() -> Layout {
    let constraints =
        PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false);
    let descriptors = ["layers", "color", "properties", "history"]
        .into_iter()
        .map(|key| (PanelKey::new(key).unwrap(), constraints))
        .collect::<Vec<_>>();
    create_initial_layout(Profile::Photoshop, &descriptors, DockMetrics::default()).unwrap()
}

type LayoutSnapshot = (
    u64,
    String,
    Vec<(NodeId, String)>,
    Vec<(PanelId, String)>,
    Vec<(DockAreaId, String)>,
    Vec<(FloatingGroupId, String)>,
);

fn layout_snapshot(layout: &Layout) -> LayoutSnapshot {
    (
        layout.version(),
        format!(
            "{:?}|{:?}|{:?}|{:?}",
            layout.profile(),
            layout.chrome(),
            layout.canvas(),
            layout.flyout()
        ),
        layout
            .nodes()
            .map(|(id, node)| (id, format!("{node:?}")))
            .collect(),
        layout
            .panel_instances()
            .map(|(id, panel)| (id, format!("{panel:?}")))
            .collect(),
        layout
            .dock_areas()
            .map(|(id, area)| (id, format!("{area:?}")))
            .collect(),
        layout
            .floating_groups()
            .map(|(id, group)| (id, format!("{group:?}")))
            .collect(),
    )
}

fn target_for_layout(layout: &Layout) -> DropTarget {
    let group = layout
        .nodes()
        .find_map(|(id, node)| matches!(node, Node::Tabs(_)).then_some(id))
        .expect("baseline layout contains a tab group");
    DropTarget::Tabs { group, index: 0 }
}

#[test]
fn pointer_down_is_pressed_and_click_below_threshold_is_cancelled() {
    let payload = panel_payload();
    let mut session = session();

    session
        .pointer_down(payload, Point::new(10.0, 20.0))
        .unwrap();
    assert_eq!(
        session.state(),
        DragState::Pressed {
            payload,
            origin: Point::new(10.0, 20.0),
            position: Point::new(10.0, 20.0),
        }
    );

    session.pointer_move(Point::new(13.9, 20.0)).unwrap();
    assert!(matches!(session.state(), DragState::Pressed { .. }));

    assert_eq!(
        session
            .pointer_up(Point::new(13.5, 20.0), Some(DropTarget::FreeArea))
            .unwrap(),
        None
    );
    assert_eq!(
        session.state(),
        DragState::Cancelled {
            payload,
            origin: Point::new(10.0, 20.0),
            position: Point::new(13.5, 20.0),
            reason: DragCancelReason::BelowThreshold,
        }
    );
}

#[test]
fn non_finite_release_cancels_active_drag_instead_of_leaving_it_stuck() {
    let payload = panel_payload();
    let mut session = session();
    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(8.0, 0.0)).unwrap();

    assert_eq!(
        session.pointer_up(Point::new(f32::NAN, 0.0), Some(DropTarget::FreeArea)),
        Err(DragSessionError::InvalidPointerPosition)
    );
    assert!(matches!(
        session.state(),
        DragState::Cancelled { payload: cancelled, reason: DragCancelReason::InvalidPointerPosition, .. }
            if cancelled == payload
    ));
}

#[test]
fn non_finite_move_cancels_active_drag() {
    let payload = panel_payload();
    let mut session = session();
    session.pointer_down(payload, Point::ZERO).unwrap();
    assert_eq!(
        session.pointer_move(Point::new(0.0, f32::INFINITY)),
        Err(DragSessionError::InvalidPointerPosition)
    );
    assert!(matches!(
        session.state(),
        DragState::Cancelled {
            reason: DragCancelReason::InvalidPointerPosition,
            ..
        }
    ));
}

#[test]
fn preview_exposes_point_and_optional_target_without_mutating_layout() {
    let layout = test_layout();
    let before = layout_snapshot(&layout);
    let payload = panel_payload();
    let mut session = session();

    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 1.0)).unwrap();
    assert_eq!(
        session.preview(),
        Some(DragPreview {
            payload,
            point: Point::new(4.0, 1.0),
            target: None,
        })
    );

    let target = target_for_layout(&layout);
    session.set_drop_target(Some(target)).unwrap();
    assert_eq!(
        session.preview(),
        Some(DragPreview {
            payload,
            point: Point::new(4.0, 1.0),
            target: Some(target),
        })
    );
    assert_eq!(layout_snapshot(&layout), before);
}

#[test]
fn valid_release_emits_exactly_one_request() {
    let layout = test_layout();
    let before = layout_snapshot(&layout);
    let payload = panel_payload();
    let target = target_for_layout(&layout);
    let mut session = session();
    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 0.0)).unwrap();
    session.set_drop_target(Some(target)).unwrap();

    let request = session
        .pointer_up(Point::new(8.0, 2.0), Some(target))
        .unwrap()
        .expect("valid release emits one request");
    assert_eq!(
        request,
        DragDropRequest {
            payload,
            handle: DragHandle::PanelTab,
            origin: Point::ZERO,
            point: Point::new(8.0, 2.0),
            target: Some(target),
            floating_bounds: None,
        }
    );
    assert_eq!(
        session.state(),
        DragState::Dropped {
            payload,
            origin: Point::ZERO,
            position: Point::new(8.0, 2.0),
            target: Some(target),
        }
    );
    assert_eq!(
        session.pointer_up(Point::new(8.0, 2.0), Some(target)),
        Err(DragSessionError::NoActiveGesture),
        "a terminal gesture must not emit a second request"
    );
    assert_eq!(layout_snapshot(&layout), before);
}

#[test]
fn cancel_and_release_without_target_emit_no_request_and_preserve_layout() {
    let layout = test_layout();
    let before = layout_snapshot(&layout);
    let payload = panel_payload();
    let mut session = session();

    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 0.0)).unwrap();
    assert_eq!(
        session.pointer_up(Point::new(5.0, 0.0), None).unwrap(),
        None
    );
    assert_eq!(
        session.state(),
        DragState::Cancelled {
            payload,
            origin: Point::ZERO,
            position: Point::new(5.0, 0.0),
            reason: DragCancelReason::NoValidTarget,
        }
    );
    assert_eq!(layout_snapshot(&layout), before);

    session.reset().unwrap();
    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 0.0)).unwrap();
    session.escape().unwrap();
    assert_eq!(
        session.state(),
        DragState::Cancelled {
            payload,
            origin: Point::ZERO,
            position: Point::new(4.0, 0.0),
            reason: DragCancelReason::Escape,
        }
    );
    assert_eq!(layout_snapshot(&layout), before);
}

#[test]
fn focus_loss_cancels_without_a_drop_request() {
    let payload = panel_payload();
    let mut session = session();
    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 0.0)).unwrap();

    session.focus_lost().unwrap();
    assert!(matches!(
        session.state(),
        DragState::Cancelled {
            reason: DragCancelReason::FocusLost,
            ..
        }
    ));
    assert_eq!(
        session.pointer_up(Point::new(4.0, 0.0), None),
        Err(DragSessionError::NoActiveGesture)
    );
}

#[test]
fn release_position_can_cross_threshold_without_an_intermediate_move_event() {
    let payload = panel_payload();
    let mut session = session();

    session.pointer_down(payload, Point::ZERO).unwrap();
    let request = session
        .pointer_up(Point::new(0.0, 4.0), Some(DropTarget::FreeArea))
        .unwrap()
        .expect("release point crosses threshold");
    assert_eq!(request.point, Point::new(0.0, 4.0));
    assert_eq!(request.target, Some(DropTarget::FreeArea));
}

#[test]
fn threshold_uses_euclidean_logical_distance_and_is_configurable() {
    let metrics = DockMetrics {
        drag_threshold: 5.0,
        ..DockMetrics::default()
    };
    let mut session = DragSession::new(&metrics).unwrap();

    assert_eq!(session.threshold(), 5.0);
    let payload = panel_payload();
    session.pointer_down(payload, Point::ZERO).unwrap();

    session.pointer_move(Point::new(3.0, 3.0)).unwrap();
    assert!(matches!(session.state(), DragState::Pressed { .. }));

    session.pointer_move(Point::new(3.0, 4.0)).unwrap();
    assert!(matches!(session.state(), DragState::Dragging { .. }));
    assert_eq!(session.preview().unwrap().target, None);
}

#[test]
fn cancellation_works_before_and_after_drag_and_reset_reuses_session() {
    let payload = panel_payload();
    let mut session = session();

    session.pointer_down(payload, Point::ZERO).unwrap();
    session.cancel().unwrap();
    assert!(matches!(
        session.state(),
        DragState::Cancelled {
            reason: DragCancelReason::Explicit,
            ..
        }
    ));

    session.reset().unwrap();
    assert_eq!(session.state(), DragState::Idle);

    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 0.0)).unwrap();
    session.cancel().unwrap();
    assert!(matches!(session.state(), DragState::Cancelled { .. }));

    session.reset().unwrap();
    assert_eq!(session.state(), DragState::Idle);
}

#[test]
fn moving_clears_a_stale_target_and_target_cannot_be_set_before_drag() {
    let payload = panel_payload();
    let mut session = session();
    assert_eq!(
        session.set_drop_target(Some(DropTarget::FreeArea)),
        Err(DragSessionError::NotDragging)
    );

    session.pointer_down(payload, Point::ZERO).unwrap();
    session.pointer_move(Point::new(4.0, 0.0)).unwrap();
    session.set_drop_target(Some(DropTarget::FreeArea)).unwrap();
    session.pointer_move(Point::new(4.5, 0.0)).unwrap();
    assert_eq!(session.preview().unwrap().target, None);
}

#[test]
fn invalid_positions_threshold_and_transitions_are_rejected_with_valid_state_transitions() {
    let metrics = DockMetrics {
        drag_threshold: f32::NAN,
        ..DockMetrics::default()
    };
    assert_eq!(
        DragSession::new(&metrics),
        Err(DragSessionError::InvalidThreshold)
    );

    let mut session = session();
    assert_eq!(
        session.pointer_down(panel_payload(), Point::new(f32::NAN, 0.0)),
        Err(DragSessionError::InvalidPointerPosition)
    );
    assert_eq!(session.state(), DragState::Idle);
    assert_eq!(
        session.pointer_move(Point::ZERO),
        Err(DragSessionError::NoActiveGesture)
    );

    let payload = panel_payload();
    session.pointer_down(payload, Point::ZERO).unwrap();

    // An invalid pointer position terminates the active gesture so the
    // adapter cannot accidentally keep or commit a stale drag preview.
    assert_eq!(
        session.pointer_move(Point::new(f32::INFINITY, 0.0)),
        Err(DragSessionError::InvalidPointerPosition)
    );
    assert!(matches!(
        session.state(),
        DragState::Cancelled {
            payload: cancelled,
            origin: Point::ZERO,
            position: Point::ZERO,
            reason: DragCancelReason::InvalidPointerPosition,
        } if cancelled == payload
    ));
    assert_eq!(
        session.pointer_move(Point::ZERO),
        Err(DragSessionError::NoActiveGesture)
    );
    assert_eq!(
        session.pointer_down(payload, Point::ZERO),
        Err(DragSessionError::SessionNotIdle)
    );

    session.reset().unwrap();
    assert_eq!(session.state(), DragState::Idle);
}
