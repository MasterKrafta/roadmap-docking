/// Stable, user-facing key used to persist a registered panel in a layout file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelKey(String);

impl PanelKey {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("panel key must not be empty");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use super::PanelKey;

    #[test]
    fn accepts_builtin_and_new_panel_keys_without_an_enum() {
        for value in [
            "layers",
            "properties",
            "color",
            "history",
            "custom.adjustments",
        ] {
            let key = PanelKey::new(value).expect("non-empty panel key should be accepted");
            assert_eq!(key.as_str(), value);
        }
    }

    #[test]
    fn supports_equality_hashing_and_total_ordering() {
        let layers = PanelKey::new("layers").expect("valid key");
        let same_layers = PanelKey::new("layers").expect("valid key");
        let properties = PanelKey::new("properties").expect("valid key");

        assert_eq!(layers, same_layers);
        assert!(layers < properties);

        let mut ordered = BTreeSet::new();
        ordered.insert(properties.clone());
        ordered.insert(layers.clone());
        assert_eq!(
            ordered.into_iter().collect::<Vec<_>>(),
            vec![layers.clone(), properties.clone()]
        );

        let mut hashed = HashSet::new();
        hashed.insert(layers.clone());
        hashed.insert(same_layers);
        hashed.insert(properties);
        assert_eq!(hashed.len(), 2);
    }

    #[test]
    fn rejects_empty_keys() {
        assert!(PanelKey::new("").is_err());
        assert!(PanelKey::new("   ").is_err());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn implements_serde_traits_when_feature_is_enabled() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<PanelKey>();
    }
}
