use crate::{DragCancelReason, DragPayload, DropTarget, Point};

/// State of one UI-independent pointer drag gesture.
///
/// All positions are in logical pixels. This is transient interaction state,
/// not part of the persistent layout model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragState {
    /// No gesture is active.
    Idle,
    /// Pointer is down, but movement has not crossed the drag threshold.
    Pressed {
        payload: DragPayload,
        origin: Point,
        position: Point,
    },
    /// Movement crossed the threshold; target is refreshed by the resolver
    /// after pointer movement. The Layout itself remains untouched.
    Dragging {
        payload: DragPayload,
        origin: Point,
        position: Point,
        target: Option<DropTarget>,
    },
    /// A valid target was present when the pointer was released.
    Dropped {
        payload: DragPayload,
        origin: Point,
        position: Point,
        target: DropTarget,
    },
    /// Gesture ended without a drop request.
    Cancelled {
        payload: DragPayload,
        origin: Point,
        position: Point,
        reason: DragCancelReason,
    },
}
