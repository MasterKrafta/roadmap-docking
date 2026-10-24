pub mod components;
mod chrome_geometry;
mod color;
mod frame;
mod geometry;
mod labels;
mod metrics;
mod render;
mod tree_query;
pub mod panels;

pub use frame::FrameGeometry;
pub use geometry::union_rects;
pub use metrics::dock_metrics_from_theme;
pub use chrome_geometry::normalize_docked_group_geometry;
pub use render::{
    CommandClickHandler, DockMenuRequest, DragOverlay, DragStartHandler, ResizeHandle,
    ResizeStartHandler, render, render_frame_from_registry, render_from_registry,
};
