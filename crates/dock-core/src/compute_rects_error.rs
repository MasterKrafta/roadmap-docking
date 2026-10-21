use crate::{Axis, GeometryError};

/// A failure while resolving model layout into logical-pixel rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputeRectsError {
    /// Dock metrics failed their consistency checks.
    InvalidMetrics,
    /// The model failed layout validation before any rectangles were emitted.
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
            Self::InvalidMetrics => formatter.write_str("dock metrics are invalid"),
            Self::InvalidLayout => formatter.write_str("layout is invalid"),
            Self::Geometry(error) => write!(formatter, "invalid geometry: {error:?}"),
            Self::InsufficientCanvasWidth { available, minimum } => write!(
                formatter,
                "Canvas width {available} is below the minimum {minimum}"
            ),
            Self::InsufficientSpace { axis, available, required } => write!(
                formatter,
                "insufficient {axis:?}-axis space: {available} available, {required} required"
            ),
        }
    }
}

impl std::error::Error for ComputeRectsError {}
