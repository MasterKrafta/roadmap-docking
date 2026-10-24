pub mod components;
pub mod host;
pub mod requests;
pub mod surface;
mod chrome_geometry;
mod color;
mod entry;
mod frame;
mod geometry;
mod labels;
mod layers;
mod main_window;
mod metrics;
mod render;
mod tree_query;
pub mod panels;

pub use chrome_geometry::normalize_docked_group_geometry;
pub use entry::{RunOptions, run};
pub use frame::FrameGeometry;
pub use geometry::union_rects;
pub use host::{CommandClickHandler, DockHost, DragStartHandler, NoopHost, ResizeStartHandler};
pub use metrics::dock_metrics_from_theme;
pub use render::{
    render, render_frame_from_registry, render_frame_with_host, render_from_registry,
    render_with_host,
};
pub use requests::{DockMenuRequest, DragOverlay, ResizeHandle};
pub use surface::DockSurface;
