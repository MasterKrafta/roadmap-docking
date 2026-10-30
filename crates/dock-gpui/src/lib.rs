#![forbid(unsafe_code)]

//! GPUI adapter for the UI-independent dock-core layout engine.
//!
//! The controller owns one dock-core Layout. UI components translate input,
//! dispatch semantic commands, and render geometry snapshots produced by Core.

pub mod controller;
pub mod registry;

/// The UI-independent docking model used by this adapter.
pub use dock_core;

use std::path::PathBuf;

/// Options for starting the dock application.
#[derive(Default)]
pub struct RunOptions {
    /// Optional workspace file to restore at startup.
    pub workspace_path: Option<PathBuf>,
}

/// Temporary entry point retained while the GPUI host is being implemented.
pub fn run(_options: RunOptions) {
    tracing::warn!("dock-gpui host is not implemented yet");
}

pub use controller::{
    DockController, DockControllerConfigError, DockEvent, DragController, PanelCommandError,
    ResizeInteraction, ResizeInteractionError,
};
pub use registry::{IconSource, PanelDescriptor, PanelRegistry, PanelRegistryError};

pub use dock_core::{
    AllowedArea, Axis, CanvasState, ChromeMode, Column, ColumnDisplayItem, ColumnMode, ColumnRect,
    CommandError, CommandOutcome, CommandValue, ComputeRectsError, DEFAULT_UNDO_LIMIT, DockArea,
    InitialLayoutError,
    DockAreaId, DockAreaKind, DockCommand, DockMetrics, DockSide, DragCancelReason, DragDropAction,
    DragDropRequest, DragHandle, DragPayload, DragPreview, DragSession, DragSessionError, DragState,
    DropSource, DropTarget, DropZone, DropZoneRect, DropZoneResolver, FloatingGroup,
    FloatingGroupId, FloatingGroupRect, FloatingResizeEdgeRect, FlyoutRect, FlyoutState,
    GeometryError, GroupRect, HitResult, LastDockLocation, Layout, LayoutRects, Node, NodeId, Panel,
    PanelConstraints, PanelId, PanelInstance, PanelKey, PanelKeyError, PanelRect, PanelState, Point,
    Profile, Rect, ResizeEdge, Size, Split, SplitterRect, TabHeaderRect, TabOverflowRect,
    TabStripRect, Tabs, UndoRedoHistory, ValidationError, ValidationReason, VerticalOverflow,
    WorkspaceDiagnostic, WorkspaceFile, WorkspaceFileError, WorkspaceFloatingGroup,
    WorkspaceLayout, WorkspaceLoadOptions, WorkspaceLoadResult, WorkspaceNode,
};
