use crate::{PanelConstraints, PanelKey};

/// Core-owned metadata for one instance of a panel implementation.
///
/// The instance is stored in Layout's panel arena under a PanelId. Panel
/// content state remains owned by the application or panel implementation.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelInstance {
    pub key: PanelKey,
    pub constraints: PanelConstraints,
}

impl PanelInstance {
    pub const fn new(key: PanelKey, constraints: PanelConstraints) -> Self {
        Self { key, constraints }
    }
}
