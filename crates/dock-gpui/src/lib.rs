mod metrics;
mod render;
pub mod panels;

pub use metrics::dock_metrics_from_theme;
pub use render::{
    CommandClickHandler, DockMenuRequest, DragOverlay, DragStartHandler, ResizeHandle,
    ResizeStartHandler, normalize_docked_group_geometry, render, render_from_registry,
};
