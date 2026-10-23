/// Selects the layout rules applied by the core model.
///
/// The selected profile is explicit and is persisted in the workspace; it is
/// never inferred from the shape of the current tree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Profile {
    /// General-purpose docking without Photoshop-specific tree restrictions.
    Generic,
    /// Photoshop-style columns, panel groups, collapse, and flyout behavior.
    Photoshop,
}

#[cfg(test)]
mod tests {
    use super::Profile;

    #[test]
    fn profiles_are_explicitly_distinct() {
        assert_ne!(Profile::Generic, Profile::Photoshop);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_profile() {
        for profile in [Profile::Generic, Profile::Photoshop] {
            let encoded = serde_json::to_string(&profile).unwrap();
            assert_eq!(serde_json::from_str::<Profile>(&encoded).unwrap(), profile);
        }
    }
}
