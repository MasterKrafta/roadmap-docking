#![forbid(unsafe_code)]

//! UI-independent data model and behavior for docking layouts.

pub mod dock_area_id;
pub mod dock_area_kind;
pub mod dock_profile;
pub mod dock_side;
pub mod floating_group_id;
pub mod node_id;
pub mod panel;
pub mod panel_id;
pub mod panel_instance;
pub mod panel_key;
pub mod split;
pub mod split_axis;
pub mod tabs;

pub use dock_area_id::DockAreaId;
pub use dock_area_kind::DockAreaKind;
pub use dock_profile::DockProfile;
pub use dock_side::DockSide;
pub use floating_group_id::FloatingGroupId;
pub use node_id::NodeId;
pub use panel::Panel;
pub use panel_id::PanelId;
pub use panel_instance::PanelInstance;
pub use panel_key::PanelKey;
pub use split::Split;
pub use split_axis::SplitAxis;
pub use tabs::Tabs;
