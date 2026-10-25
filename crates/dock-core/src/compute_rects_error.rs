use crate::{Axis, GeometryError, ValidationError};

/// A failure while resolving model layout into logical-pixel rectangles.
#[derive(Clone, Debug, PartialEq)]
pub enum ComputeRectsError {
    /// Dock metrics failed their consistency checks.
    InvalidMetrics { reason: &'static str },
    /// Full diagnostics from model validation before any rectangles were emitted.
    LayoutValidation { errors: Vec<ValidationError> },
    /// The model's internal structure is invalid where geometry cannot proceed.
    InvalidLayout,
    /// The supplied viewport or an intermediate rectangle was invalid.
    Geometry(GeometryError),
    /// Dock widths would leave less than the configured minimum Canvas width.
    InsufficientCanvasWidth { available: f32, minimum: f32 },
    /// A split or group stack cannot satisfy its minimum sizes in this viewport.
    InsufficientSpace {
        axis: Axis,
        available: f32,
        required: f32,
    },
}

impl std::fmt::Display for ComputeRectsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMetrics { reason } => {
                write!(formatter, "dock metrics are invalid: {reason}")
            }
            Self::LayoutValidation { errors } => {
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
            Self::InvalidLayout => formatter.write_str("layout structure is invalid"),
            Self::Geometry(error) => write!(formatter, "invalid geometry: {error}"),
            Self::InsufficientCanvasWidth { available, minimum } => write!(
                formatter,
                "Canvas width {available} is below the minimum {minimum}"
            ),
            Self::InsufficientSpace {
                axis,
                available,
                required,
            } => write!(
                formatter,
                "insufficient {axis:?}-axis space: {available} available, {required} required"
            ),
        }
    }
}

impl std::error::Error for ComputeRectsError {}

#[cfg(test)]
mod tests {
    use super::ComputeRectsError;
    use crate::{GeometryError, ValidationError};

    #[test]
    fn display_uses_geometry_error_message_instead_of_debug_format() {
        assert_eq!(
            ComputeRectsError::Geometry(GeometryError::NonFinite).to_string(),
            "invalid geometry: geometry contains a non-finite value"
        );
    }

    #[test]
    fn display_preserves_layout_validation_reasons() {
        let error = ComputeRectsError::LayoutValidation {
            errors: vec![ValidationError::InvalidCanvas {
                reason: "stored canvas bounds are invalid",
            }],
        };
        assert_eq!(
            error.to_string(),
            "layout is invalid: invalid canvas state: stored canvas bounds are invalid"
        );
    }

    #[test]
    fn display_preserves_metric_validation_reason() {
        let error = ComputeRectsError::InvalidMetrics {
            reason: "min_canvas_width must be positive",
        };
        assert_eq!(
            error.to_string(),
            "dock metrics are invalid: min_canvas_width must be positive"
        );
    }
}
