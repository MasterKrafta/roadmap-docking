use std::collections::{HashMap, VecDeque};

use dock_core::{
    CommandError, DockMetrics, DragSession, DragSessionError, DropSource, DropZoneResolver,
    FlyoutState, GeometryError, HitResult, Layout, LayoutRects, PanelId, PanelKey, Point, Profile,
    Rect, ValidationError, WorkspaceDiagnostic,
};
use gpui::{AnyView, EventEmitter};
use thiserror::Error;

use crate::registry::PanelRegistry;

mod commands;
mod geometry;

/// Events emitted by the controller entity after successful model operations.
#[derive(Clone, Debug)]
pub enum DockEvent {
    LayoutChanged { version: u64 },
    PanelOpened { panel: PanelId, key: PanelKey },
    PanelClosed { panel: PanelId, key: PanelKey },
    PanelActivated { group: dock_core::NodeId, panel: PanelId },
    CommandFailed(CommandError),
    WorkspaceLoaded { diagnostics: Vec<WorkspaceDiagnostic> },
}

impl EventEmitter<DockEvent> for DockController {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RectCacheKey {
    layout_version: u64,
    viewport: Rect,
    metrics_revision: u64,
    chrome: dock_core::ChromeMode,
    flyout: Option<FlyoutState>,
    /// ResizeFlyout is transient and does not advance Layout::version().
    /// Its remembered width therefore participates in the geometry cache key.
    flyout_width: Option<f32>,
}

/// Controller construction and configuration errors.
#[derive(Debug, Error)]
pub enum DockControllerConfigError {
    #[error("invalid dock metrics: {0}")]
    InvalidMetrics(&'static str),

    #[error("initial layout is invalid for the supplied metrics: {0:?}")]
    InvalidLayout(Vec<ValidationError>),

    #[error(transparent)]
    InvalidGeometry(#[from] GeometryError),

    #[error(transparent)]
    InvalidDragSession(#[from] DragSessionError),

    #[error("dock metrics revision overflow")]
    MetricsRevisionOverflow,
}

/// Errors returned by panel-oriented controller helpers.
#[derive(Debug, Error)]
pub enum PanelCommandError {
    #[error("panel '{0}' is not registered")]
    UnknownPanel(PanelKey),

    #[error(transparent)]
    Command(#[from] CommandError),
}

/// State for an active drag gesture.
///
/// DragSession and the optional resolver are transient interaction data only;
/// the layout tree remains exclusively owned by dock_core::Layout.
pub struct DragController {
    session: DragSession,
    resolver: Option<DropZoneResolver>,
    resolver_key: Option<RectCacheKey>,
    resolver_source: Option<DropSource>,
}

impl DragController {
    fn new(metrics: &DockMetrics) -> Result<Self, DragSessionError> {
        Ok(Self {
            session: DragSession::new(metrics)?,
            resolver: None,
            resolver_key: None,
            resolver_source: None,
        })
    }

    pub fn session(&self) -> &DragSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut DragSession {
        &mut self.session
    }

    pub fn resolver(&self) -> Option<&DropZoneResolver> {
        self.resolver.as_ref()
    }

    fn set_resolver(
        &mut self,
        resolver: DropZoneResolver,
        key: RectCacheKey,
        source: DropSource,
    ) {
        self.resolver = Some(resolver);
        self.resolver_key = Some(key);
        self.resolver_source = Some(source);
    }

    pub fn clear_resolver(&mut self) {
        self.resolver = None;
        self.resolver_key = None;
        self.resolver_source = None;
    }

    /// Cancels any active gesture and clears the resolver snapshot.
    ///
    /// Used when the controller replaces its Layout lineage, because runtime
    /// IDs carried by an in-progress drag must never cross into a new Layout.
    pub fn cancel_gesture(&mut self) {
        let _ = self.session.focus_lost();
        let _ = self.session.reset();
        self.clear_resolver();
    }
}

/// The semantic target and pointer origin captured at the start of one resize.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeInteraction {
    pub hit: HitResult,
    pub origin: Point,
    pub layout_version: u64,
    pub metrics_revision: u64,
}

/// Invalid attempts to begin a resize interaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum ResizeInteractionError {
    #[error("a resize interaction is already active")]
    AlreadyActive,

    #[error("the hit result does not identify a resize handle")]
    NotResizeHandle,

    #[error(transparent)]
    InvalidPointer(#[from] GeometryError),
}

/// Owns the sole dock-core layout and the adapter state needed to operate it.
///
/// DockController is intended to live inside a GPUI Entity. It contains no
/// duplicate tree: every structural read and mutation goes through Layout.
pub struct DockController {
    layout: Layout,
    history: dock_core::UndoRedoHistory,
    metrics: DockMetrics,
    viewport: Rect,
    metrics_revision: u64,
    rects: Option<LayoutRects>,
    rects_key: Option<RectCacheKey>,
    drag: DragController,
    interaction: Option<ResizeInteraction>,
    registry: PanelRegistry,
    panel_views: HashMap<PanelId, AnyView>,
    last_error: Option<CommandError>,
    pending_events: VecDeque<DockEvent>,
}

impl DockController {
    /// Creates a controller around an existing, validated core layout.
    pub fn new(
        layout: Layout,
        metrics: DockMetrics,
        registry: PanelRegistry,
        viewport: Rect,
    ) -> Result<Self, DockControllerConfigError> {
        metrics
            .validate()
            .map_err(DockControllerConfigError::InvalidMetrics)?;
        viewport.validate_non_empty()?;
        layout
            .validate(&metrics)
            .map_err(DockControllerConfigError::InvalidLayout)?;

        Ok(Self {
            layout,
            history: dock_core::UndoRedoHistory::new(),
            drag: DragController::new(&metrics)?,
            metrics,
            viewport,
            metrics_revision: 0,
            rects: None,
            rects_key: None,
            interaction: None,
            registry,
            panel_views: HashMap::new(),
            last_error: None,
            pending_events: VecDeque::new(),
        })
    }

    /// Creates an empty workspace for the requested profile.
    pub fn empty(
        profile: Profile,
        metrics: DockMetrics,
        registry: PanelRegistry,
        viewport: Rect,
    ) -> Result<Self, DockControllerConfigError> {
        Self::new(Layout::new(profile), metrics, registry, viewport)
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn metrics(&self) -> DockMetrics {
        self.metrics
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn registry(&self) -> &PanelRegistry {
        &self.registry
    }

    pub fn drag(&self) -> &DragController {
        &self.drag
    }

    pub fn drag_mut(&mut self) -> &mut DragController {
        &mut self.drag
    }

    pub fn resize_interaction(&self) -> Option<ResizeInteraction> {
        self.interaction
    }

    /// Starts tracking a resize from a Core hit-test result.
    pub fn begin_resize_interaction(
        &mut self,
        hit: HitResult,
        origin: Point,
    ) -> Result<(), ResizeInteractionError> {
        if self.interaction.is_some() {
            return Err(ResizeInteractionError::AlreadyActive);
        }
        origin.validate()?;
        if !matches!(
            hit,
            HitResult::Splitter { .. }
                | HitResult::FlyoutResizeEdge { .. }
                | HitResult::FloatingResizeEdge { .. }
        ) {
            return Err(ResizeInteractionError::NotResizeHandle);
        }
        self.drag.clear_resolver();
        self.interaction = Some(ResizeInteraction {
            hit,
            origin,
            layout_version: self.layout.version(),
            metrics_revision: self.metrics_revision,
        });
        Ok(())
    }

    /// Ends the active resize and returns its captured start data.
    pub fn end_resize_interaction(&mut self) -> Option<ResizeInteraction> {
        self.interaction.take()
    }

    pub fn last_error(&self) -> Option<&CommandError> {
        self.last_error.as_ref()
    }

    pub fn clear_last_error(&mut self) {
        self.last_error = None;
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
