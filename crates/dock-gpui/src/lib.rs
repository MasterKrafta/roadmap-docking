#![forbid(unsafe_code)]

//! GPUI adapter for the UI-independent dock-core layout engine.
//!
//! The controller owns one dock-core Layout. UI components translate input,
//! dispatch semantic commands, and render geometry snapshots produced by Core.

pub mod actions;
pub mod components;
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

use gpui::{Context, Window, div, prelude::*};

/// Options for starting the built-in dock demo.
#[derive(Default)]
pub struct RunOptions {
    /// Optional workspace file to restore instead of the default demo layout.
    pub workspace_path: Option<PathBuf>,
}

/// Small content view used by the built-in launcher until an application
/// supplies its own PanelRegistry and panel factories.
struct StarterPanel {
    title: &'static str,
}

impl Render for StarterPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_4().child(self.title)
    }
}

fn starter_panel_registry() -> PanelRegistry {
    use dock_core::{PanelConstraints, PanelKey, Size};

    let mut registry = PanelRegistry::new();
    let constraints = PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(280.0, 220.0),
        None,
        false,
    );

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

/// Launches the built-in Photoshop-style workspace on Windows.
///
/// The default workspace uses four lightweight starter panels. When the
/// workspace_path option is set, the saved workspace is loaded before the main
/// window is created; a load error is logged and the default workspace remains
/// available.
#[cfg(windows)]
pub fn run(options: RunOptions) {
    use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;

    let workspace_path = options.workspace_path;

    application().run(move |cx: &mut App| {
        let metrics = DockMetrics::default();
        let registry = starter_panel_registry();
        let descriptors = registry.descriptors_for_core();
        let layout = match dock_core::create_initial_layout(
            Profile::Photoshop,
            &descriptors,
            metrics,
        ) {
            Ok(layout) => layout,
            Err(error) => {
                tracing::error!(%error, "could not create the initial dock layout");
                return;
            }
        };

        let viewport = Rect::new(Point::ZERO, Size::new(1440.0, 900.0));
        let mut controller = match DockController::new(layout, metrics, registry, viewport) {
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
                "could not restore the requested dock workspace; using the default layout"
            );
        }

        let controller = cx.new(|_| controller);
        let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
        let open_result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| cx.new(|_| DockHost::new(controller.clone(), DockTheme::default())),
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

pub use components::{
    CanvasSlot, ColumnView, ContextMenu, ContextMenuEntry, ContextMenuItem, DragGhost, DropOverlay,
    FloatingHost, FlyoutView, GroupView, MainDockView, PanelHost, RailView,
    SplitterView, TabHeaderView, TabOverflow, TabStripView,
};
pub use dock_host::DockHost;
pub use floating_manager::{FloatingWindowManager, PanelWindowOwner, WindowRegistry};
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
    FloatingGroupId, FloatingGroupRect, FlyoutRect, FlyoutState,
    GeometryError, GroupRect, HitResult, InitialLayoutError, LastDockLocation, Layout, Node,
    NodeId, OUTER_COLUMN_EDGE_BOUNDARY_INDEX, Panel, PanelConstraints, PanelId, PanelInstance,
    PanelKey, PanelKeyError, PanelRect, PanelState, Point, Profile, Rect, ResizeEdge, Size, Split,
    SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, Tabs, UndoRedoHistory,
    ValidationError, ValidationReason, VerticalOverflow, WorkspaceDiagnostic, WorkspaceFile,
    WorkspaceFileError, WorkspaceFloatingGroup, WorkspaceLayout, WorkspaceLoadOptions,
    WorkspaceLoadResult, WorkspaceNode,
};
