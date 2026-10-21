use crate::{ColumnMode, NodeId};

/// A profile-specific column containing an ordered vertical sequence of groups.
///
/// Groups refer to Tabs nodes in the Photoshop profile. A non-empty column
/// remains a Column even when it contains only one group.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Column {
    pub width: f32,
    pub mode: ColumnMode,
    pub groups: Vec<NodeId>,
    pub group_ratios: Vec<f32>,
    /// Last user-selected width while expanded; rail width is presentation-only.
    #[cfg_attr(feature = "serde", serde(default))]
    pub last_expanded_width: Option<f32>,
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::Column;

    #[test]
    fn missing_last_expanded_width_defaults_to_none() {
        let legacy = r#"{"width":280.0,"mode":"Expanded","groups":[],"group_ratios":[]}"#;
        let column: Column = serde_json::from_str(legacy).unwrap();
        assert_eq!(column.last_expanded_width, None);
    }
}
