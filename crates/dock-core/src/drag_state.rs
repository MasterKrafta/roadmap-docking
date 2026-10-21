use crate::{DragPayload, Point};

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
    /// Movement crossed the threshold; the payload may be resolved to drop
    /// zones by the caller.
    Dragging {
        payload: DragPayload,
        origin: Point,
        position: Point,
    },
    /// Pointer was released after the drag threshold was crossed.
    Dropped {
        payload: DragPayload,
        origin: Point,
        position: Point,
    },
    /// Gesture was cancelled, including a press/release that never crossed
    /// the threshold (a click is not a drag).
    Cancelled {
        payload: DragPayload,
        origin: Point,
        position: Point,
    },
}
