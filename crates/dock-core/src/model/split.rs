use super::Axis;
use crate::ids::NodeId;
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Split {
    pub axis: Axis,
    pub children: Vec<NodeId>,
    pub ratios: Vec<f32>,
}
