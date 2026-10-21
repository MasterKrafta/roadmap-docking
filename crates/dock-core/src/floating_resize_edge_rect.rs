use crate::{FloatingGroupId, Rect, ResizeEdge};

/// Hit region for one edge or corner resize handle of a floating group.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingResizeEdgeRect {
    pub group: FloatingGroupId,
    pub edge: ResizeEdge,
    pub rect: Rect,
}

impl FloatingResizeEdgeRect {
    pub const fn new(group: FloatingGroupId, edge: ResizeEdge, rect: Rect) -> Self {
        Self { group, edge, rect }
    }
}
