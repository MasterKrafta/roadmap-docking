use dock_core::{NodeId, PanelId, Point};

#[derive(Clone, Copy)]
pub(super) struct FloatingTabPress {
    pub(super) tab_group: NodeId,
    pub(super) panel: PanelId,
    pub(super) origin: Point,
    pub(super) layout_version: u64,
}
