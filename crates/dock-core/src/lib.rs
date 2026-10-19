#![forbid(unsafe_code)]

pub mod geometry;
pub mod ids;
pub mod model;

pub use geometry::DockMetrics;
pub use ids::{FloatingGroupId, NodeId, PanelId, PanelKey};
pub use model::Profile;
