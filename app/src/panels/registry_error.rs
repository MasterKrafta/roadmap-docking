use std::{error::Error, fmt};

use dock_core::PanelKey;

use super::AllowedArea;

/// Errors reported by panel descriptor validation and registry operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    DuplicateKey(PanelKey),
    UnknownPanel(PanelKey),
    InvalidDescriptor {
        key: PanelKey,
        reason: &'static str,
    },
    AreaNotAllowed {
        key: PanelKey,
        area: AllowedArea,
    },
    UnsupportedOpenTarget {
        key: PanelKey,
        target: &'static str,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey(key) => write!(formatter, "panel key is already registered: {key:?}"),
            Self::UnknownPanel(key) => write!(formatter, "panel key is not registered: {key:?}"),
            Self::InvalidDescriptor { key, reason } => {
                write!(formatter, "invalid descriptor for panel {key:?}: {reason}")
            }
            Self::AreaNotAllowed { key, area } => {
                write!(formatter, "panel {key:?} does not allow placement in {area:?}")
            }
            Self::UnsupportedOpenTarget { key, target } => {
                write!(formatter, "OpenPanel does not support target {target} for panel {key:?}")
            }
        }
    }
}

impl Error for RegistryError {}
