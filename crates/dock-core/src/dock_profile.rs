/// Selects the layout rules applied by the core model.
///
/// The selected profile is explicit; it must not be inferred from tree shape.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DockProfile {
    /// General-purpose docking with no Photoshop-specific tree restrictions.
    Generic,
    /// Photoshop-style columns, panel groups, collapse, and flyout behavior.
    Photoshop,
}

#[cfg(test)]
mod tests {
    use super::DockProfile;

    #[test]
    fn profiles_are_explicitly_distinct() {
        assert_ne!(DockProfile::Generic, DockProfile::Photoshop);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_the_selected_profile() {
        for profile in [DockProfile::Generic, DockProfile::Photoshop] {
            let encoded = serde_json::to_string(&profile).unwrap();
            assert_eq!(serde_json::from_str::<DockProfile>(&encoded).unwrap(), profile);
        }
    }
}
