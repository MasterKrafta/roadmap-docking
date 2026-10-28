use crate::{NodeId, PanelId, Rect};

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

/// Presentation mode of a Photoshop-style dock column.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ColumnMode {
    /// Full-width column with expanded group headers and contents.
    Expanded,
    /// Compact rail showing each panel icon and its label. The fixed width is
    /// `DockMetrics::min_column_width`.
    ///
    /// Groups and tabs are flattened for presentation only; the model tree is
    /// kept intact so expanding the column restores the same tab stacks.
    IconsAndLabels,
    /// Narrow icon-only rail with width `DockMetrics::rail_width`.
    ///
    /// Groups and tabs are flattened for presentation only; the model tree is
    /// kept intact so expanding the column restores the same tab stacks.
    IconsOnly,
}

impl ColumnMode {
    /// Returns whether this mode uses the flattened rail presentation.
    pub const fn is_rail(self) -> bool {
        matches!(self, Self::IconsAndLabels | Self::IconsOnly)
    }
}

/// One panel entry in the canonical flattened display order of a column.
///
/// Rail adapters enumerate groups top-to-bottom and each group's tabs
/// left-to-right. This is presentation metadata only: it does not rewrite the
/// column tree, tab order, or active-tab selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnDisplayItem {
    /// Tabs group that owns this tab.
    pub group: NodeId,
    /// Panel represented by this entry.
    pub panel: PanelId,
    /// Tab position within its owning group.
    pub tab_index: usize,
    /// Zero-based position in the flattened column order.
    pub display_index: usize,
}

impl ColumnDisplayItem {
    pub const fn new(
        group: NodeId,
        panel: PanelId,
        tab_index: usize,
        display_index: usize,
    ) -> Self {
        Self {
            group,
            panel,
            tab_index,
            display_index,
        }
    }
}

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

#[cfg(all(test, feature = "serde"))]
mod column_serde_tests {
    use super::Column;

    #[test]
    fn missing_last_expanded_width_defaults_to_none() {
        let legacy = r#"{"width":280.0,"mode":"Expanded","groups":[],"group_ratios":[]}"#;
        let column: Column = serde_json::from_str(legacy).unwrap();
        assert_eq!(column.last_expanded_width, None);
    }
}

#[cfg(test)]
mod column_mode_tests {
    use super::ColumnMode;

    #[test]
    fn all_column_modes_are_distinct_and_rail_modes_are_explicit() {
        assert_ne!(ColumnMode::Expanded, ColumnMode::IconsAndLabels);
        assert_ne!(ColumnMode::Expanded, ColumnMode::IconsOnly);
        assert_ne!(ColumnMode::IconsAndLabels, ColumnMode::IconsOnly);
        assert!(!ColumnMode::Expanded.is_rail());
        assert!(ColumnMode::IconsAndLabels.is_rail());
        assert!(ColumnMode::IconsOnly.is_rail());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn all_column_modes_round_trip_through_serde() {
        for mode in [
            ColumnMode::Expanded,
            ColumnMode::IconsAndLabels,
            ColumnMode::IconsOnly,
        ] {
            let encoded = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<ColumnMode>(&encoded).unwrap(), mode);
        }
    }
}
