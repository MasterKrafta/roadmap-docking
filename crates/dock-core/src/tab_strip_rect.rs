use crate::{NodeId, Rect};

/// Geometry of the tab-strip region belonging to one Tabs group.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TabStripRect {
    pub group: NodeId,
    pub rect: Rect,
}

impl TabStripRect {
    pub const fn new(group: NodeId, rect: Rect) -> Self {
        Self { group, rect }
    }
}
