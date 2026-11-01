#![forbid(unsafe_code)]

//! GPUI adapter for the UI-independent dock-core layout engine.
//!
//! The controller owns one dock-core Layout. UI components translate input,
//! dispatch semantic commands, and render geometry snapshots produced by Core.

pub mod actions;
pub mod components;
mod app_shell;
pub mod controller;
pub mod dock_host;
pub mod floating_manager;

mod geometry;
pub mod input;
pub mod missing_panel;
pub mod persistence;
pub mod registry;
pub mod theme;

/// The UI-independent docking model used by this adapter.
pub use dock_core;

use std::path::PathBuf;

#[cfg(windows)]
use gpui::{AppContext, Context, Window, div, prelude::*};

/// Options for starting the built-in dock demo.
#[derive(Default)]
pub struct RunOptions {
    /// Optional workspace file to restore instead of starting with an empty workspace.
    pub workspace_path: Option<PathBuf>,
}

/// Small content view used by the built-in launcher. Applications that need
/// their own panels can construct DockController and DockHost directly.
#[cfg(windows)]
struct StarterPanel {
    title: &'static str,
}

#[cfg(windows)]
impl Render for StarterPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_4().child(self.title)
    }
}

#[cfg(windows)]
fn starter_panel_registry() -> PanelRegistry {
    use dock_core::{PanelConstraints, PanelKey, Size};

    let mut registry = PanelRegistry::new();
    let constraints =
        PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false);

    for (key, title) in [
        ("layers", "Layers"),
        ("color", "Color"),
        ("properties", "Properties"),
        ("history", "History"),
    ] {
        let panel_key = PanelKey::new(key).expect("built-in panel keys must be valid");
        registry
            .register(PanelDescriptor::new(
                panel_key,
                title,
                IconSource::None,
                constraints,
                move |_window, app| app.new(|_| StarterPanel { title }).into(),
            ))
            .expect("built-in panel descriptors must be valid");
    }

    registry
}

/// Launches the built-in empty Dock workspace on Windows.
///
/// The normal launch creates only the main window. Panel types remain available
/// in the registry, but no panel instances or panel windows are created until a
/// workspace is restored or the user explicitly opens a panel. When the
/// workspace_path option is set, the saved workspace is loaded before the main
/// window is created; a load error is logged and the workspace remains empty.
#[cfg(windows)]
pub fn run(options: RunOptions) {
    use gpui::{App, Bounds, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;

    let workspace_path = options.workspace_path;

    application().run(move |cx: &mut App| {
        let metrics = DockMetrics::default();
        let registry = starter_panel_registry();
        let viewport = Rect::new(Point::ZERO, Size::new(1440.0, 740.0));
        let mut controller =
            match DockController::empty(Profile::Photoshop, metrics, registry, viewport) {
                Ok(controller) => controller,
                Err(error) => {
                    tracing::error!(%error, "could not create the dock controller");
                    return;
                }
            };

        if let Some(path) = workspace_path.as_ref()
            && let Err(error) = controller.load_workspace_from(path)
        {
            tracing::warn!(
                path = %path.display(),
                %error,
                "could not restore the requested dock workspace; keeping the workspace empty"
            );
        }

        let controller = cx.new(|_| controller);
        let bounds = Bounds::centered(None, size(px(1440.0), px(740.0)), cx);
        let theme = DockTheme::default();
        let open_result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                let manager_controller = controller.clone();
                let manager =
                    cx.new(move |_| FloatingWindowManager::new(manager_controller, theme));

                let host_controller = controller.clone();
                let host_manager = manager.clone();
                let dock_host = cx.new(move |_| {
                    DockHost::new(host_controller, theme).with_floating_window_manager(host_manager)
                });

                cx.new(move |_| AppShell::new(controller, dock_host, manager, theme))
            },
        );

        if open_result.is_err() {
            tracing::error!("GPUI could not open the main dock window");
            return;
        }

        cx.activate(true);
    });
}

/// The current native floating-window implementation is Windows-specific.
#[cfg(not(windows))]
pub fn run(_options: RunOptions) {
    tracing::warn!("the built-in dock demo currently requires the Windows native-window backend");
}

pub use app_shell::AppShell;
pub use components::{
    CanvasSlot, ColumnView, ContextMenu, ContextMenuEntry, ContextMenuItem, DragGhost, DropOverlay,
    FloatingHost, FlyoutView, GroupView, MainDockView, PanelHost, RailView, SplitterView,
    TabHeaderView, TabOverflow, TabStripView,
};
pub use dock_host::DockHost;
pub use floating_manager::{
    CompositionHostRegistry, FloatingWindowManager, PanelWindowOwner, PanelWindowRegistry,
    PanelWindowVisibility, WindowRegistry,
};
pub use missing_panel::MissingPanelPlaceholder;
pub use persistence::WorkspacePersistenceError;
pub use theme::DockTheme;

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
    FloatingGroupId, FloatingGroupRect, FlyoutRect, FlyoutState, GeometryError, GroupRect,
    HitResult, InitialLayoutError, LastDockLocation, Layout, Node, NodeId,
    OUTER_COLUMN_EDGE_BOUNDARY_INDEX, Panel, PanelConstraints, PanelId, PanelInstance, PanelKey,
    PanelKeyError, PanelRect, PanelState, Point, Profile, Rect, ResizeEdge, Size, Split,
    SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, Tabs, UndoRedoHistory,
    ValidationError, ValidationReason, VerticalOverflow, WorkspaceDiagnostic, WorkspaceFile,
    WorkspaceFileError, WorkspaceFloatingGroup, WorkspaceLayout, WorkspaceLoadOptions,
    WorkspaceLoadResult, WorkspaceNode,
};
