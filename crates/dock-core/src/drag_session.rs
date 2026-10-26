#[path = "drag_session/pointer.rs"]
mod pointer;
#[path = "drag_session/terminal.rs"]
mod terminal;

use crate::{DragHandle, DragState};

/// A deterministic, UI-independent drag gesture state machine.
///
/// The threshold is copied from `DockMetrics::drag_threshold` at creation,
/// so it is configurable and stable for the lifetime of this gesture session.
/// Pointer positions and the threshold are measured in logical pixels.
///
/// A session never owns or mutates `Layout`. It exposes preview data while
/// dragging and emits at most one semantic `DragDropRequest` on valid release.
#[derive(Clone, Debug, PartialEq)]
pub struct DragSession {
    state: DragState,
    threshold: f32,
    handle: Option<DragHandle>,
}

#[cfg(test)]
#[path = "drag_session/tests.rs"]
mod tests;
