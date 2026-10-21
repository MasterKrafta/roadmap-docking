/// Presentation mode of a Photoshop-style dock column.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ColumnMode {
    /// Full-width column with expanded group headers and contents.
    Expanded,
    /// Compact rail showing each panel icon and its label.
    ///
    /// Groups and tabs are flattened for presentation only; the model tree is
    /// kept intact so expanding the column restores the same tab stacks.
    IconsAndLabels,
    /// Narrow icon-only rail.
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

#[cfg(test)]
mod tests {
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
