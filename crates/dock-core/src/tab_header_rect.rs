use crate::{NodeId, PanelId, Rect};

/// Geometry of one tab header and its position within the Tabs group.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TabHeaderRect {
    pub group: NodeId,
    pub panel: PanelId,
    pub index: usize,
    pub rect: Rect,
}

impl TabHeaderRect {
    pub const fn new(group: NodeId, panel: PanelId, index: usize, rect: Rect) -> Self {
        Self { group, panel, index, rect }
    }
}
