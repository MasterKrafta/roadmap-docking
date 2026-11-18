use dock_core::NodeId;

#[derive(Clone, Copy)]
pub(super) struct FloatingGroupPress {
    pub(super) group: NodeId,
    pub(super) layout_version: u64,
}
