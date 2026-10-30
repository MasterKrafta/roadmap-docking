use dock_core::{DragHandle, DragPayload, DragState, Point};
use gpui::{App, Entity, MouseDownEvent, MouseMoveEvent, MouseUpEvent};

use crate::DockController;

/// Begins a Core drag session from a semantic panel or group source.
pub(crate) fn begin_drag(
    controller: &Entity<DockController>,
    payload: DragPayload,
    handle: DragHandle,
    event: &MouseDownEvent,
    app: &mut App,
) {
    let position = event_point(event.position.x, event.position.y);
    if let Err(error) = controller.update(app, |controller, _| {
        controller.drag_mut().session_mut().pointer_down_with_handle(payload, handle, position)
    }) {
        tracing::debug!(%error, "could not begin dock drag gesture");
    }
}

/// Updates the shared drag state machine while the pointer is held.
pub(crate) fn update_drag(
    controller: &Entity<DockController>,
    event: &MouseMoveEvent,
    app: &mut App,
) {
    if !event.dragging() {
        return;
    }
    let position = event_point(event.position.x, event.position.y);
    let _ = controller.update(app, |controller, _| {
        let session = controller.drag_mut().session_mut();
        if matches!(
            session.state(),
            DragState::Pressed { .. } | DragState::Dragging { .. }
        ) {
            let _ = session.pointer_move(position);
        }
    });
}

/// Ends a gesture. Drop-target resolution is added by the drag adapter stage.
pub(crate) fn finish_drag(
    controller: &Entity<DockController>,
    event: &MouseUpEvent,
    app: &mut App,
) {
    let position = event_point(event.position.x, event.position.y);
    let _ = controller.update(app, |controller, _| {
        let session = controller.drag_mut().session_mut();
        if matches!(
            session.state(),
            DragState::Pressed { .. } | DragState::Dragging { .. }
        ) {
            let _ = session.pointer_up(position, None);
            let _ = session.reset();
        }
    });
}

fn event_point(x: gpui::Pixels, y: gpui::Pixels) -> Point {
    Point::new(x.as_f32(), y.as_f32())
}
