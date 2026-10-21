use crate::{NodeId, Rect};

/// Geometry of one Tabs group in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupRect {
    pub group: NodeId,
    pub rect: Rect,
}

impl GroupRect {
    pub const fn new(group: NodeId, rect: Rect) -> Self {
        Self { group, rect }
    }
}
