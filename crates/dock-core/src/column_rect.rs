use crate::{NodeId, Rect};

/// Geometry of a column node in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnRect {
    pub column: NodeId,
    pub rect: Rect,
}

impl ColumnRect {
    pub const fn new(column: NodeId, rect: Rect) -> Self {
        Self { column, rect }
    }
}
