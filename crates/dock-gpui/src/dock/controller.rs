use std::collections::HashMap;

use dock_core::{DockMetrics, Layout, NodeId};
use gpui::ScrollHandle;

/// Owns the live layout and the view-local state needed to render it.
///
/// Command history and interaction sessions are added in their corresponding
/// Stage 7 checklist items; the layout ownership boundary is established here.
pub(crate) struct DockController {
    layout: Layout,
    metrics: DockMetrics,
    scroll_handles: HashMap<NodeId, ScrollHandle>,
}

impl DockController {
    pub(crate) fn new(layout: Layout, metrics: DockMetrics) -> Self {
        Self {
            layout,
            metrics,
            scroll_handles: HashMap::new(),
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
