use dock_core::{DockMetrics, Layout, UndoRedoHistory};
use ui::Theme;

use super::rail_policy::RailPolicy;

/// Persistent workspace state, separated from one window's input and viewport state.
///
/// The layout and undo history belong together. Pointer sessions, scroll handles,
/// and viewport measurements remain in DockController because they are specific
/// to an individual rendered view.
pub(crate) struct WorkspaceModel {
    pub(super) layout: Layout,
    pub(super) metrics: DockMetrics,
    pub(super) rail_policy: Option<RailPolicy>,
    pub(super) history: UndoRedoHistory,
}

impl WorkspaceModel {
    pub(super) fn new(theme: &Theme, layout: Layout, metrics: DockMetrics) -> Self {
        Self {
            layout,
            metrics,
            rail_policy: RailPolicy::new(
                theme.metrics.rail_collapse_width,
                theme.metrics.rail_expand_width,
            ),
            history: UndoRedoHistory::new(),
        }
    }
}
