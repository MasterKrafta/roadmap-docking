#![forbid(unsafe_code)]

//! UI-independent data model and behavior for docking layouts.

mod allowed_area;
mod axis;
mod canvas_state;
mod chrome_mode;
mod column;
pub(crate) mod command_application;
mod command_error;
mod command_outcome;
mod command_value;
mod compute_drop_zones;
mod compute_rects;
mod compute_rects_error;
mod dock_area;
mod dock_area_id;
mod dock_area_kind;
mod dock_command;
mod dock_metrics;
mod dock_side;
mod drag_types;
mod drag_session;
mod drop_types;
mod floating_group;
mod floating_group_id;
mod floating_group_rect;
mod floating_resize_edge_rect;
mod flyout_rect;
mod flyout_state;
mod geometry_error;
mod group_rect;
mod hit_test;
mod last_dock_location;
mod layout;
mod layout_factory;
mod layout_rects;
mod node;
mod node_id;
mod panel;
mod panel_instance;
mod panel_key;
mod point;
mod profile;
mod rect;
mod resize_edge;
mod size;
mod split;
mod splitter_rect;
mod tab_header_rect;
mod tab_overflow_rect;
mod tab_strip_rect;
mod tabs;
mod undo_redo_history;
mod validation_error;
mod vertical_overflow;
#[cfg(feature = "serde")]
mod workspace_types;
#[cfg(feature = "serde")]
mod workspace_file;

pub use allowed_area::AllowedArea;
pub use axis::Axis;
pub use canvas_state::CanvasState;
pub use chrome_mode::ChromeMode;
pub use column::{Column, ColumnDisplayItem, ColumnMode, ColumnRect};
pub use command_error::CommandError;
pub use command_outcome::CommandOutcome;
pub use command_value::CommandValue;
pub use compute_rects_error::ComputeRectsError;
pub use dock_area::DockArea;
pub use dock_area_id::DockAreaId;
pub use dock_area_kind::DockAreaKind;
pub use dock_command::DockCommand;
pub use dock_metrics::DockMetrics;
pub use dock_side::DockSide;
pub use drag_types::{
    DragCancelReason, DragDropAction, DragDropRequest, DragHandle, DragPayload, DragPreview,
    DragSessionError, DragState,
};
pub use drag_session::DragSession;
pub use drop_types::{DropSource, DropTarget, DropZone, DropZoneRect};
pub use compute_drop_zones::DropZoneResolver;
pub use floating_group::FloatingGroup;
pub use floating_group_id::FloatingGroupId;
pub use floating_group_rect::FloatingGroupRect;
pub use floating_resize_edge_rect::FloatingResizeEdgeRect;
pub use flyout_rect::FlyoutRect;
pub use flyout_state::FlyoutState;
pub use geometry_error::GeometryError;
pub use group_rect::GroupRect;
pub use hit_test::HitResult;
pub use last_dock_location::LastDockLocation;
pub use layout::Layout;
pub use layout_factory::{create_initial_layout, InitialLayoutError};
pub use layout_rects::LayoutRects;
pub use node::Node;
pub use node_id::NodeId;
pub use panel::{Panel, PanelId, PanelRect};
pub use panel_instance::{PanelConstraints, PanelInstance, PanelState};
pub use panel_key::{PanelKey, PanelKeyError};
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
pub use workspace_file::{WORKSPACE_FORMAT, WORKSPACE_VERSION, WorkspaceFile};
#[cfg(feature = "serde")]
pub use workspace_types::{
    WorkspaceDiagnostic, WorkspaceFileError, WorkspaceFloatingGroup, WorkspaceLayout,
    WorkspaceLoadOptions, WorkspaceLoadResult, WorkspaceNode,
};

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
