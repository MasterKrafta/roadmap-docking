use crate::model::DockSide;

/// Identifies a column by dock side and its current zero-based position.
///
/// Columns do not have persistent IDs in the model yet, so this reference is
/// intended for immediate commands, not for long-lived saved layout data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnRef {
    pub side: DockSide,
    pub index: usize,
}

impl ColumnRef {
    pub const fn new(side: DockSide, index: usize) -> Self {
        Self { side, index }
    }
}
