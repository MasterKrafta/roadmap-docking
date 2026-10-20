use super::{Column, DockSide};
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DockArea {
    pub side: DockSide,
    pub columns: Vec<Column>,
}
