#![forbid(unsafe_code)]

//! UI-independent data model and behavior for docking layouts.

pub mod axis;
pub mod chrome_mode;
pub mod column;
pub mod column_mode;
pub mod dock_area_id;
pub mod dock_area_kind;
pub mod dock_metrics;
pub mod dock_side;
pub mod floating_group_id;
pub mod geometry_error;
pub mod node;
pub mod node_id;
pub mod panel;
pub mod panel_constraints;
pub mod panel_id;
pub mod panel_instance;
pub mod panel_key;
pub mod panel_state;
pub mod point;
pub mod profile;
pub mod rect;
pub mod size;
pub mod split;
pub mod tabs;

pub use axis::Axis;
pub use chrome_mode::ChromeMode;
pub use column::Column;
pub use column_mode::ColumnMode;
pub use dock_area_id::DockAreaId;
pub use dock_area_kind::DockAreaKind;
pub use dock_metrics::DockMetrics;
pub use dock_side::DockSide;
pub use floating_group_id::FloatingGroupId;
pub use geometry_error::GeometryError;
pub use node::Node;
pub use node_id::NodeId;
pub use panel::Panel;
pub use panel_constraints::PanelConstraints;
pub use panel_id::PanelId;
pub use panel_instance::PanelInstance;
pub use panel_key::PanelKey;
pub use panel_state::PanelState;
pub use point::Point;
pub use profile::Profile;
pub use rect::Rect;
pub use size::Size;
pub use split::Split;
pub use tabs::Tabs;
