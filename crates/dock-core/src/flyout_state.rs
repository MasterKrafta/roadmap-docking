use crate::NodeId;

/// Transient reference to a group opened from a collapsed column rail.
///
/// This state is intentionally not serialized and does not create or modify
/// persistent tree nodes by itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlyoutState {
    pub column: NodeId,
    pub group: NodeId,
}

impl FlyoutState {
    pub const fn new(column: NodeId, group: NodeId) -> Self {
        Self { column, group }
    }
}
