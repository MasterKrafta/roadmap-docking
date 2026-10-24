use std::collections::HashMap;

use dock_core::{
    CommandError, CommandOutcome, ComputeRectsError, DockCommand, DockMetrics, Layout, NodeId,
    Rect, UndoRedoHistory,
};
use gpui::ScrollHandle;

use crate::FrameGeometry;

/// Owns the live layout and the view-local state needed to render it.
///
/// All post-initialization layout commands are applied through the history
/// facade, keeping command execution and undo checkpoints on one path.
pub(crate) struct DockController {
    layout: Layout,
    metrics: DockMetrics,
    history: UndoRedoHistory,
    scroll_handles: HashMap<NodeId, ScrollHandle>,
    last_error: Option<CommandError>,
}

impl DockController {
    pub(crate) fn new(layout: Layout, metrics: DockMetrics) -> Self {
        Self {
            layout,
            metrics,
            history: UndoRedoHistory::new(),
            scroll_handles: HashMap::new(),
            last_error: None,
        }
    }

    /// Applies a layout command and records persistent changes for undo/redo.
    pub(crate) fn apply(
        &mut self,
        command: DockCommand,
    ) -> Result<CommandOutcome, CommandError> {
        let result = self
            .history
            .apply(&mut self.layout, command, &self.metrics);
        self.track_command_result(result, "apply")
    }

    /// Restores the preceding recorded layout revision, if one exists.
    pub(crate) fn undo(&mut self) -> Result<bool, CommandError> {
        let result = self.history.undo(&mut self.layout, &self.metrics);
        self.track_command_result(result, "undo")
    }

    /// Reapplies the next recorded layout revision, if one exists.
    pub(crate) fn redo(&mut self) -> Result<bool, CommandError> {
        let result = self.history.redo(&mut self.layout, &self.metrics);
        self.track_command_result(result, "redo")
    }

    /// Computes the normalized frame geometry for the current layout and viewport.
    pub(crate) fn frame(&self, viewport: Rect) -> Result<FrameGeometry, ComputeRectsError> {
        FrameGeometry::compute(&self.layout, viewport, &self.metrics)
    }

    /// Returns the most recent command/history error, if the last operation failed.
    pub(crate) fn last_error(&self) -> Option<&CommandError> {
        self.last_error.as_ref()
    }

    fn track_command_result<T>(
        &mut self,
        result: Result<T, CommandError>,
        operation: &'static str,
    ) -> Result<T, CommandError> {
        match result {
            Ok(value) => {
                self.last_error = None;
                Ok(value)
            }
            Err(error) => {
                tracing::warn!(
                    operation,
                    error = %error,
                    "dock controller operation failed"
                );
                self.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn layout(&self) -> &Layout {
        &self.layout
    }

    pub(crate) fn metrics(&self) -> &DockMetrics {
        &self.metrics
    }

    pub(crate) fn scroll_handles(&self) -> &HashMap<NodeId, ScrollHandle> {
        &self.scroll_handles
    }

    pub(crate) fn scroll_handles_mut(&mut self) -> &mut HashMap<NodeId, ScrollHandle> {
        &mut self.scroll_handles
    }
}
