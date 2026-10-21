use crate::{Axis, NodeId};

/// An ordered split of child nodes along one axis.
///
/// The invariant of at least two children and one valid ratio per child is
/// enforced by layout validation/normalization, so candidate states can be
/// represented before they are accepted.
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub axis: Axis,
    pub children: Vec<NodeId>,
    pub ratios: Vec<f32>,
}
