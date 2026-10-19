use crate::ids::NodeId;
use super::ColumnMode;
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Column { pub width: f32, pub mode: ColumnMode, pub groups: Vec<NodeId>, pub group_ratios: Vec<f32> }
