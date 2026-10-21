use crate::{Axis, GeometryError, ValidationError};

/// Errors returned by parsing, migrating, and recovering the versioned workspace JSON boundary.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceFileError {
    #[error("invalid workspace JSON: {0}")]
    Json(String),

    #[error("unsupported workspace format '{found}'")]
    UnsupportedFormat { found: String },

    #[error("workspace version {found} is unsupported; the current schema version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("invalid dock metrics: {0}")]
    InvalidMetrics(&'static str),

    #[error("duplicate panel descriptor key '{key}'")]
    DuplicatePanelDescriptor { key: String },

    #[error("panel descriptor '{key}' has invalid constraints: {reason:?}")]
    InvalidPanelDescriptor {
        key: String,
        reason: GeometryError,
    },

    #[error("floating group {index} has invalid bounds: {reason:?}")]
    InvalidFloatingBounds { index: usize, reason: GeometryError },

    #[error("floating group {index} cannot fit in the visible client region: {axis:?} has {available} available, {required} required")]
    FloatingBoundsInsufficientSpace {
        index: usize,
        axis: Axis,
        available: f32,
        required: f32,
    },

    #[error("visible client region has invalid bounds: {0:?}")]
    InvalidVisibleClientRegion(GeometryError),

    #[error("cannot create workspace file from layout: {0}")]
    InvalidRuntimeLayout(String),

    #[error("workspace layout violates core invariants: {0:?}")]
    InvalidLayout(Vec<ValidationError>),
}
