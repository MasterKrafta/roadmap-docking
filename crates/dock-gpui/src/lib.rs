#![forbid(unsafe_code)]

//! GPUI adapter for the UI-independent dock-core layout engine.
//!
//! The controller owns one dock-core Layout. UI components translate input,
//! dispatch semantic commands, and render geometry snapshots produced by Core.

pub mod actions;
pub mod components;
pub mod controller;
pub mod dock_host;

mod geometry;
pub mod input;
pub mod missing_panel;
pub mod persistence;
pub mod registry;
pub mod theme;
pub mod workspace;

/// The UI-independent docking model used by this adapter.
pub use dock_core;

/// Options for starting the dock application.
#[derive(Default)]
pub struct RunOptions {
    /// Optional workspace file to restore at startup.
    pub workspace_path: Option<std::path::PathBuf>,
}

/// Temporary entry point retained while the GPUI host is being implemented.
pub fn run(_options: RunOptions) {
    tracing::warn!("dock-gpui host is not implemented yet");
}

pub use components::{
    CanvasSlot, ColumnView, ContextMenu, ContextMenuCallbacks, ContextMenuEntry, ContextMenuItem,
    DragGhost, DropOverlay,
    FloatingWindowView, FlyoutView, GroupView, MainDockView, PanelHost, RailView, SplitterView,
    TabHeaderView, TabOverflow, TabStripView,
};
pub use dock_host::DockHost;
pub use missing_panel::MissingPanelPlaceholder;
pub use persistence::WorkspacePersistenceError;
pub use theme::DockTheme;
pub use workspace::{DockPanelMenu, DockWorkspace, DockWorkspaceError, PanelDefinition};

pub use controller::{
    DockController, DockControllerConfigError, DockEvent, DragController, PanelCommandError,
    ResizeInteraction, ResizeInteractionError,
};
pub use registry::{IconSource, PanelDescriptor, PanelRegistry, PanelRegistryError};

pub use dock_core::{
    AllowedArea, Axis, CanvasState, ChromeMode, Column, ColumnDisplayItem, ColumnMode, ColumnRect,
    CommandError, CommandOutcome, CommandValue, ComputeRectsError, DEFAULT_UNDO_LIMIT, DockArea,
    DockAreaId, DockAreaKind, DockCommand, DockMetrics, DockSide, DragCancelReason, DragDropAction,
    DragDropRequest, DragHandle, DragPayload, DragPreview, DragSession, DragSessionError,
    DragState, DropSource, DropTarget, DropZone, DropZoneRect, DropZoneResolver, FloatingGroup,
    FloatingGroupId, FloatingGroupRect, FloatingResizeEdgeRect, FlyoutRect, FlyoutState,
    GeometryError, GroupRect, HitResult, InitialLayoutError, LastDockLocation, Layout, Node,
    NodeId, OUTER_COLUMN_EDGE_BOUNDARY_INDEX, Panel, PanelConstraints, PanelId, PanelInstance,
    PanelKey, PanelKeyError, PanelRect, PanelState, Point, Profile, Rect, ResizeEdge, Size, Split,
    SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, Tabs, UndoRedoHistory,
    ValidationError, ValidationReason, VerticalOverflow, WorkspaceDiagnostic, WorkspaceFile,
    WorkspaceFileError, WorkspaceFloatingGroup, WorkspaceLayout, WorkspaceLoadOptions,
    WorkspaceLoadResult, WorkspaceNode,
};
