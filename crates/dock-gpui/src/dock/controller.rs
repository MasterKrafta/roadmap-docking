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
}

impl DockController {
    pub(crate) fn new(layout: Layout, metrics: DockMetrics) -> Self {
        Self {
            layout,
            metrics,
            history: UndoRedoHistory::new(),
            scroll_handles: HashMap::new(),
        }
    }

    /// Applies a layout command and records persistent changes for undo/redo.
    pub(crate) fn apply(
        &mut self,
        command: DockCommand,
    ) -> Result<CommandOutcome, CommandError> {
        self.history
            .apply(&mut self.layout, command, &self.metrics)
    }

    /// Restores the preceding recorded layout revision, if one exists.
    pub(crate) fn undo(&mut self) -> Result<bool, CommandError> {
        self.history.undo(&mut self.layout, &self.metrics)
    }

    /// Reapplies the next recorded layout revision, if one exists.
    pub(crate) fn redo(&mut self) -> Result<bool, CommandError> {
        self.history.redo(&mut self.layout, &self.metrics)
    }

    /// Computes the normalized frame geometry for the current layout and viewport.
    pub(crate) fn frame(&self, viewport: Rect) -> Result<FrameGeometry, ComputeRectsError> {
        FrameGeometry::compute(&self.layout, viewport, &self.metrics)
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
