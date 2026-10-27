//! Thin adapter from a completed pointer gesture to dock-core's semantic drop action.
//!
//! Core owns the source/target validation and the mapping to a single DockCommand.
//! Keep that policy in one place instead of duplicating its decision table in GPUI.

use dock_core::{CommandError, DockMetrics, DragDropAction, DragDropRequest, Layout};

/// Resolves a released drag without mutating the layout.
///
/// `DragDropAction::Command` contains the one command to commit through
/// `DockController` history; `NoOp` deliberately produces no history entry.
pub(super) fn resolve_drop(
    layout: &Layout,
    request: &DragDropRequest,
    metrics: &DockMetrics,
) -> Result<DragDropAction, CommandError> {
    layout.resolve_drag_drop(request, metrics)
}
