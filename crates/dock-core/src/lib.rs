#![forbid(unsafe_code)]

//! UI-independent data model and behavior for docking layouts.

pub mod dock_area_id;
pub mod floating_group_id;
pub mod node_id;
pub mod panel_id;
pub mod panel_key;

pub use dock_area_id::DockAreaId;
pub use floating_group_id::FloatingGroupId;
pub use node_id::NodeId;
pub use panel_id::PanelId;
pub use panel_key::PanelKey;
