use gpui::AnyView;

use dock_core::{DockMetrics, GroupRect, NodeId, PanelRect, TabHeaderRect, TabOverflowRect, TabStripRect};

use crate::dock_components::GroupPresentation;

pub(super) struct DragGroupPresentation {
    pub(super) group: GroupRect,
    pub(super) strip: TabStripRect,
    pub(super) headers: Vec<TabHeaderRect>,
    pub(super) overflow: Option<TabOverflowRect>,
    pub(super) presentation: GroupPresentation,
    pub(super) column: Option<NodeId>,
    pub(super) panel: Option<(PanelRect, AnyView)>,
    pub(super) metrics: DockMetrics,
}
