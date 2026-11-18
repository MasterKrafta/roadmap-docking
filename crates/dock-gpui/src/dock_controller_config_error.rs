use dock_core::{DragSessionError, GeometryError, ValidationError};
use thiserror::Error;

/// Controller construction and configuration errors.
#[derive(Debug, Error)]
pub enum DockControllerConfigError {
    #[error("invalid dock metrics: {0}")]
    InvalidMetrics(&'static str),

    #[error("initial layout is invalid for the supplied metrics: {0:?}")]
    InvalidLayout(Vec<ValidationError>),

    #[error(transparent)]
    InvalidGeometry(#[from] GeometryError),

    #[error(transparent)]
    InvalidDragSession(#[from] DragSessionError),

    #[error("dock metrics cannot change during an active pointer, native move, or DragHost transfer")]
    ActiveInteraction,

    #[error("dock metrics revision overflow")]
    MetricsRevisionOverflow,
}
