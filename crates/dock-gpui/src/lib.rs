pub mod components;
pub mod host;
pub mod requests;
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
pub use host::{CommandClickHandler, DockHost, DragStartHandler, NoopHost, ResizeStartHandler};
pub use requests::{DockMenuRequest, DragOverlay, ResizeHandle};
pub use render::{
    render, render_frame_from_registry, render_frame_with_host, render_from_registry,
    render_with_host,
};
