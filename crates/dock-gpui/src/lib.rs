//! Reusable GPUI docking surface, host integration contract, and a reference window entrypoint.
//!
//! Embedded applications provide their own [`host::DockHost`] to render domain-specific
//! panel content. The bundled [`run`] window is a reference/demo integration; its panel
//! bodies intentionally use the placeholder content in [`panels`]. See `docs/panel-rendering.md`.
mod chrome_geometry;
mod color;
pub mod components;
mod dock;
mod entry;
mod frame;
mod floating_window;
mod geometry;
pub mod host;
mod labels;
mod layers;
mod main_window;
mod next_dock;
mod menus;
mod metrics;
pub mod panels;
mod render;
pub mod requests;
pub mod surface;
mod tree_query;

pub use chrome_geometry::normalize_docked_group_geometry;
pub use entry::{RunOptions, run};
pub use next_dock::run as run_next_dock;
pub use frame::FrameGeometry;
pub use geometry::union_rects;
pub use host::{CommandClickHandler, DockHost, DragStartHandler, NoopHost, ResizeStartHandler};
pub use metrics::dock_metrics_from_theme;
pub use render::{render_frame_with_host, render_with_host};
pub use requests::{DockMenuRequest, DragOverlay, ResizeHandle};
pub use surface::DockSurface;
