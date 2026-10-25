use std::fmt;

use crate::{ComputeRectsError, ValidationError};

/// Detailed failure from geometry computation.
///
/// This additive API preserves the underlying validation diagnostics while
/// leaving the legacy, copyable `ComputeRectsError` available for callers
/// that only need broad error categories.
#[derive(Clone, Debug, PartialEq)]
pub enum ComputeRectsDiagnosticError {
    /// Dock metrics failed their consistency checks.
    InvalidMetrics { reason: &'static str },
    /// The model failed validation before any rectangles were emitted.
    InvalidLayout { errors: Vec<ValidationError> },
    /// Geometry calculation failed after validation.
    Computation(ComputeRectsError),
}

impl fmt::Display for ComputeRectsDiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetrics { reason } => {
                write!(formatter, "dock metrics are invalid: {reason}")
            }
            Self::InvalidLayout { errors } => {
                if errors.is_empty() {
                    return formatter.write_str("layout is invalid");
                }
                write!(
                    formatter,
                    "layout is invalid: {}",
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            }
            Self::Computation(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for ComputeRectsDiagnosticError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Computation(error) => Some(error),
            Self::InvalidMetrics { .. } | Self::InvalidLayout { .. } => None,
        }
    }
}

impl From<ComputeRectsError> for ComputeRectsDiagnosticError {
    fn from(error: ComputeRectsError) -> Self {
        Self::Computation(error)
    }
}
