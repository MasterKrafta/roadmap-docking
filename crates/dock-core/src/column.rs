use crate::{ColumnMode, NodeId};

/// A profile-specific column containing an ordered vertical sequence of groups.
///
/// Groups refer to Tabs nodes in the Photoshop profile. A non-empty column
/// remains a Column even when it contains only one group.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub width: f32,
    pub mode: ColumnMode,
    pub groups: Vec<NodeId>,
    pub group_ratios: Vec<f32>,
    /// Last user-selected width while expanded; rail width is presentation-only.
    #[cfg_attr(feature = "serde", serde(default))]
    pub last_expanded_width: Option<f32>,
}
