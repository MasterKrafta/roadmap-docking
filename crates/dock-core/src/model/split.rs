use crate::ids::NodeId;
use super::Axis;
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Split { pub axis: Axis, pub children: Vec<NodeId>, pub ratios: Vec<f32> }
