use dock_core::FloatingGroupId;

#[derive(Clone, Copy)]
pub(super) struct FloatingGroupSnapshot {
    pub(super) id: FloatingGroupId,
    pub(super) z_order: i32,
}
