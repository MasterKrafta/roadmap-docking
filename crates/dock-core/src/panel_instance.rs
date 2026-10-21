use crate::PanelKey;

/// Core-owned metadata for one instance of a panel implementation.
///
/// Instance identity is the PanelId key in the layout's panel arena.
/// Panel content state remains owned by the application or panel implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PanelInstance {
    pub key: PanelKey,
}

impl PanelInstance {
    pub fn new(key: PanelKey) -> Self {
        Self { key }
    }
}
