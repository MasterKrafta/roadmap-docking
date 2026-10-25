use std::{fmt, str::FromStr};

/// A typed reason why a panel key does not satisfy the stable-key syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PanelKeyError {
    /// The key is empty.
    #[error("panel key must not be empty")]
    Empty,
    /// The key contains unsupported characters or separators.
    #[error("panel key must contain lowercase ASCII letters/digits separated by single hyphens")]
    InvalidSyntax,
}

impl PanelKeyError {
    const fn as_static_str(&self) -> &'static str {
        match self {
            Self::Empty => "panel key must not be empty",
            Self::InvalidSyntax => {
                "panel key must contain lowercase ASCII letters/digits separated by single hyphens"
            }
        }
    }
}

/// Stable identifier for a panel kind, not a runtime panel instance.
///
/// Accepted keys are non-empty lowercase ASCII segments separated by single
/// hyphens, for example layers or asset-browser.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PanelKey(String);

impl PanelKey {
    /// Creates a panel key using the legacy static-string error for source compatibility.
    ///
    /// New callers that need to inspect the failure should use PanelKey::try_new.
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        Self::try_new(value).map_err(|error| error.as_static_str())
    }

    /// Creates a panel key and preserves the validation failure as a typed error.
    pub fn try_new(value: impl Into<String>) -> Result<Self, PanelKeyError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PanelKeyError::Empty);
        }

        let valid = value.split('-').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        });
        if !valid {
            return Err(PanelKeyError::InvalidSyntax);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for PanelKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for PanelKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for PanelKey {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for PanelKey {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for PanelKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for PanelKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::PanelKey;

    #[test]
    fn accepts_builtin_and_custom_keys() {
        for key in [
            "layers",
            "properties",
            "color",
            "history",
            "asset-browser",
            "panel2",
        ] {
            assert_eq!(PanelKey::new(key).unwrap().as_str(), key);
        }
    }

    #[test]
    fn rejects_empty_or_malformed_keys() {
        for key in [
            "",
            "Layers",
            "asset browser",
            "-layers",
            "layers-",
            "two--words",
            "a_b",
            "écran",
        ] {
            assert!(PanelKey::new(key).is_err(), "accepted invalid key: {key}");
        }
    }

    #[test]
    fn try_new_reports_typed_validation_errors() {
        assert_eq!(PanelKey::try_new(""), Err(super::PanelKeyError::Empty));
        assert_eq!(
            PanelKey::try_new("Bad Key"),
            Err(super::PanelKeyError::InvalidSyntax)
        );
    }

    #[test]
    fn key_order_and_hash_are_value_based() {
        use std::collections::HashSet;
        let layers = PanelKey::new("layers").unwrap();
        let same_layers = PanelKey::new("layers").unwrap();
        let properties = PanelKey::new("properties").unwrap();
        assert_eq!(layers, same_layers);
        assert!(layers < properties);
        let keys = HashSet::from([layers, same_layers, properties]);
        assert_eq!(keys.len(), 2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_key_and_rejects_invalid_input() {
        let key = PanelKey::new("asset-browser").unwrap();
        let encoded = serde_json::to_string(&key).unwrap();
        assert_eq!(encoded, "\"asset-browser\"");
        assert_eq!(serde_json::from_str::<PanelKey>(&encoded).unwrap(), key);
        assert!(serde_json::from_str::<PanelKey>("\"Not Valid\"").is_err());
    }
}
