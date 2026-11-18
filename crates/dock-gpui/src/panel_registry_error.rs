use dock_core::{GeometryError, PanelKey};

/// Errors produced while registering panel descriptors.
#[derive(Debug, thiserror::Error)]
pub enum PanelRegistryError {
    #[error("panel key '{key}' is already registered")]
    DuplicateKey { key: PanelKey },

    #[error("panel '{key}' has invalid constraints: {source}")]
    InvalidConstraints {
        key: PanelKey,
        #[source]
        source: GeometryError,
    },
}
