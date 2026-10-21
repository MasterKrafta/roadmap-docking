use crate::{CommandError, CommandOutcome, DockCommand, DockMetrics, Layout};

/// Default number of persistent layout revisions retained for session undo/redo.
pub const DEFAULT_UNDO_LIMIT: usize = 100;

/// Session-only undo/redo history for persistent layout commands.
///
/// History stores validated in-memory Layout checkpoints rather than serializing
/// command payloads that contain runtime IDs. This restores exact logical layout
/// state (widths, ratios, tabs, column modes, panel placements and floating
/// composition). Surviving entities keep their keys; restored removed entities
/// get fresh keys so stale handles cannot become valid again. Workspace files never
/// include this history.
///
/// Records are added only when Layout::apply commits a new persistent revision.
/// No-ops, errors and transient commands such as flyout visibility do not create
/// entries. A new persistent mutation after undo discards the redo branch.
#[derive(Clone, Debug)]
pub struct UndoRedoHistory {
    undo: Vec<Layout>,
    redo: Vec<Layout>,
    limit: usize,
}

impl Default for UndoRedoHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoRedoHistory {
    /// Creates an empty history with the default limit.
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_UNDO_LIMIT)
    }

    /// Creates an empty history retaining at most limit checkpoints per stack.
    ///
    /// A limit of zero disables recording and makes undo/redo unavailable while
    /// still allowing commands to be applied through this session facade.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
        }
    }

    pub const fn limit(&self) -> usize {
        self.limit
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Applies a command and records the prior state only if a persistent
    /// revision was successfully committed.
    pub fn apply(
        &mut self,
        layout: &mut Layout,
        command: DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandOutcome, CommandError> {
        let prior_version = layout.version();
        let checkpoint = layout.history_snapshot();
        let outcome = layout.apply(command, metrics)?;

        if outcome.new_version != prior_version && self.limit > 0 {
            Self::push_bounded(&mut self.undo, checkpoint, self.limit);
            self.redo.clear();
        }

        Ok(outcome)
    }

    /// Restores the previous persistent checkpoint. Returns false when empty.
    ///
    /// Restoring advances Layout::version rather than rewinding the revision
    /// counter. If a checkpoint no longer satisfies the supplied metrics, the
    /// operation fails atomically and keeps the checkpoint on the undo stack.
    pub fn undo(
        &mut self,
        layout: &mut Layout,
        metrics: &DockMetrics,
    ) -> Result<bool, CommandError> {
        let Some(checkpoint) = self.undo.pop() else {
            return Ok(false);
        };
        let redo_checkpoint = layout.history_snapshot();

        if let Err(error) = layout.restore_history_snapshot(&checkpoint, metrics) {
            self.undo.push(checkpoint);
            return Err(error);
        }

        Self::push_bounded(&mut self.redo, redo_checkpoint, self.limit);
        Ok(true)
    }

    /// Re-applies the next checkpoint. Returns false when empty.
    pub fn redo(
        &mut self,
        layout: &mut Layout,
        metrics: &DockMetrics,
    ) -> Result<bool, CommandError> {
        let Some(checkpoint) = self.redo.pop() else {
            return Ok(false);
        };
        let undo_checkpoint = layout.history_snapshot();

        if let Err(error) = layout.restore_history_snapshot(&checkpoint, metrics) {
            self.redo.push(checkpoint);
            return Err(error);
        }

        Self::push_bounded(&mut self.undo, undo_checkpoint, self.limit);
        Ok(true)
    }

    /// Discards both stacks without changing the current layout.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn push_bounded(stack: &mut Vec<Layout>, checkpoint: Layout, limit: usize) {
        stack.push(checkpoint);
        if stack.len() > limit {
            stack.remove(0);
        }
    }
}
