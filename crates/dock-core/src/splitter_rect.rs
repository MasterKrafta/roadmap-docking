use crate::{Axis, NodeId, Rect};

/// Sentinel boundary index for the canvas-facing outer edge of a main column.
/// Ordinary splitters use a zero-based index between two adjacent children.
pub const OUTER_COLUMN_EDGE_BOUNDARY_INDEX: usize = usize::MAX;

/// Geometry and semantic boundary for one splitter hit region.
///
/// owner is the Split node for a tree split or the Column node for a vertical
/// boundary between groups. boundary_index addresses the gap between child or
/// group indices boundary_index and boundary_index + 1.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SplitterRect {
    pub owner: NodeId,
    pub boundary_index: usize,
    pub axis: Axis,
    pub rect: Rect,
}

impl SplitterRect {
    pub const fn new(owner: NodeId, boundary_index: usize, axis: Axis, rect: Rect) -> Self {
        Self {
            owner,
            boundary_index,
            axis,
            rect,
        }
    }
}
