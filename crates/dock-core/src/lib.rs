#![forbid(unsafe_code)]

//! UI-independent data model and behavior for docking layouts.

pub mod allowed_area;
pub mod axis;
pub mod canvas_state;
pub mod chrome_mode;
pub mod column;
pub mod column_display_item;
pub mod column_mode;
pub mod column_rect;
pub(crate) mod command_application;
pub mod command_error;
pub mod command_outcome;
pub mod command_value;
pub mod compute_drop_zones;
pub mod compute_rects;
pub mod compute_rects_diagnostic_error;
pub mod compute_rects_error;
pub mod dock_area;
pub mod dock_area_id;
pub mod dock_area_kind;
pub mod dock_command;
pub mod dock_metrics;
pub mod dock_side;
pub mod drag_cancel_reason;
pub mod drag_drop_action;
pub mod drag_drop_request;
pub mod drag_handle;
pub mod drag_payload;
pub mod drag_preview;
pub mod drag_session;
pub mod drag_session_error;
pub mod drag_state;
pub mod drop_source;
pub mod drop_target;
pub mod drop_zone;
pub mod drop_zone_rect;
pub mod floating_group;
pub mod floating_group_id;
pub mod floating_group_rect;
pub mod floating_resize_edge_rect;
pub mod flyout_rect;
pub mod flyout_state;
pub mod geometry_error;
pub mod group_rect;
pub mod hit_result;
pub mod hit_test;
pub mod last_dock_location;
pub mod layout;
pub mod layout_factory;
pub mod layout_rects;
pub mod node;
pub mod node_id;
pub mod panel;
pub mod panel_constraints;
pub mod panel_id;
pub mod panel_instance;
pub mod panel_key;
pub mod panel_rect;
pub mod panel_state;
pub mod point;
pub mod profile;
pub mod rect;
pub mod resize_edge;
pub mod size;
pub mod split;
pub mod splitter_rect;
pub mod tab_header_rect;
pub mod tab_overflow_rect;
pub mod tab_strip_rect;
pub mod tabs;
pub mod undo_redo_history;
pub mod validation_error;
pub mod vertical_overflow;
#[cfg(feature = "serde")]
pub mod workspace_diagnostic;
#[cfg(feature = "serde")]
pub mod workspace_file;
#[cfg(feature = "serde")]
pub mod workspace_file_error;
#[cfg(feature = "serde")]
pub mod workspace_floating_group;
#[cfg(feature = "serde")]
pub mod workspace_layout;
#[cfg(feature = "serde")]
pub mod workspace_load_options;
#[cfg(feature = "serde")]
pub mod workspace_load_result;
#[cfg(feature = "serde")]
pub mod workspace_node;

pub use allowed_area::AllowedArea;
pub use axis::Axis;
pub use canvas_state::CanvasState;
pub use chrome_mode::ChromeMode;
pub use column::Column;
pub use column_display_item::ColumnDisplayItem;
pub use column_mode::ColumnMode;
pub use column_rect::ColumnRect;
pub use command_error::CommandError;
pub use command_outcome::CommandOutcome;
pub use command_value::CommandValue;
pub use compute_rects_diagnostic_error::ComputeRectsDiagnosticError;
pub use compute_rects_error::ComputeRectsError;
pub use dock_area::DockArea;
pub use dock_area_id::DockAreaId;
pub use dock_area_kind::DockAreaKind;
pub use dock_command::DockCommand;
pub use dock_metrics::DockMetrics;
pub use dock_side::DockSide;
pub use drag_cancel_reason::DragCancelReason;
pub use drag_drop_action::DragDropAction;
pub use drag_drop_request::DragDropRequest;
pub use drag_handle::DragHandle;
pub use drag_payload::DragPayload;
pub use drag_preview::DragPreview;
pub use drag_session::DragSession;
pub use drag_session_error::DragSessionError;
pub use drag_state::DragState;
pub use drop_source::DropSource;
pub use drop_target::DropTarget;
pub use drop_zone::DropZone;
pub use drop_zone_rect::DropZoneRect;
pub use floating_group::FloatingGroup;
pub use floating_group_id::FloatingGroupId;
pub use floating_group_rect::FloatingGroupRect;
pub use floating_resize_edge_rect::FloatingResizeEdgeRect;
pub use flyout_rect::FlyoutRect;
pub use flyout_state::FlyoutState;
pub use geometry_error::GeometryError;
pub use group_rect::GroupRect;
pub use hit_result::HitResult;
pub use last_dock_location::LastDockLocation;
pub use layout::Layout;
pub use layout_factory::{
    create_initial_layout, create_initial_layout_with_error, InitialLayoutError,
};
pub use layout_rects::LayoutRects;
pub use node::Node;
pub use node_id::NodeId;
pub use panel::Panel;
pub use panel_constraints::PanelConstraints;
pub use panel_id::PanelId;
pub use panel_instance::PanelInstance;
pub use panel_key::{PanelKey, PanelKeyError};
pub use panel_rect::PanelRect;
pub use panel_state::PanelState;
pub use point::Point;
pub use profile::Profile;
pub use rect::Rect;
pub use resize_edge::ResizeEdge;
pub use size::Size;
pub use split::Split;
pub use splitter_rect::SplitterRect;
pub use tab_header_rect::TabHeaderRect;
pub use tab_overflow_rect::TabOverflowRect;
pub use tab_strip_rect::TabStripRect;
pub use tabs::Tabs;
pub use undo_redo_history::{DEFAULT_UNDO_LIMIT, UndoRedoHistory};
pub use validation_error::ValidationError;
pub use vertical_overflow::VerticalOverflow;
#[cfg(feature = "serde")]
pub use workspace_diagnostic::WorkspaceDiagnostic;
#[cfg(feature = "serde")]
pub use workspace_file::{WORKSPACE_FORMAT, WORKSPACE_VERSION, WorkspaceFile};
#[cfg(feature = "serde")]
pub use workspace_file_error::WorkspaceFileError;
#[cfg(feature = "serde")]
pub use workspace_floating_group::WorkspaceFloatingGroup;
#[cfg(feature = "serde")]
pub use workspace_layout::WorkspaceLayout;
#[cfg(feature = "serde")]
pub use workspace_load_options::WorkspaceLoadOptions;
#[cfg(feature = "serde")]
pub use workspace_load_result::WorkspaceLoadResult;
#[cfg(feature = "serde")]
pub use workspace_node::WorkspaceNode;

#[cfg(test)]
mod chrome_tests;
#[cfg(test)]
mod collapse_group_tests;
#[cfg(test)]
mod column_mode_tests;
#[cfg(test)]
mod command_sequence_tests;
#[cfg(test)]
mod drag_drop_resolution;
#[cfg(test)]
mod drag_drop_semantics;
#[cfg(test)]
mod flyout_tests;
#[cfg(test)]
mod layout_builder;
#[cfg(test)]
mod tab_overflow_tests;

#[cfg(all(test, feature = "serde"))]
mod workspace_file_tests;

#[cfg(test)]
mod history_tests;
