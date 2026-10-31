use std::collections::{HashMap, VecDeque};

use dock_core::{
    Axis, CommandError, DockCommand, DockMetrics, DragCancelReason, DragSession, DragSessionError,
    DragState, DragHandle, DragPayload, DropSource, DropZoneResolver, FlyoutState, GeometryError,
    HitResult,
    Layout, LayoutRects, Node, PanelId,
    PanelKey, Point, Profile, Rect, ValidationError, WorkspaceDiagnostic,
    OUTER_COLUMN_EDGE_BOUNDARY_INDEX,
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
    /// Layout revision captured when the current drag press began.
    layout_version: Option<u64>,
}

impl DragController {
    fn new(metrics: &DockMetrics) -> Result<Self, DragSessionError> {
        Ok(Self {
            session: DragSession::new(metrics)?,
            resolver: None,
            resolver_key: None,
            resolver_source: None,
            layout_version: None,
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

    pub(crate) fn clear_layout_version(&mut self) {
        self.layout_version = None;
    }

    /// Cancels any active gesture and clears its resolver snapshot.
    ///
    /// The adapter selects the reason so Escape, focus loss, and model/viewport
    /// invalidation remain distinguishable in Core's terminal state.
    pub fn cancel_gesture(&mut self) {
        self.cancel_gesture_with_reason(DragCancelReason::Explicit);
    }

    fn cancel_gesture_with_reason(&mut self, reason: DragCancelReason) {
        match reason {
            DragCancelReason::Escape => {
                let _ = self.session.escape();
            }
            DragCancelReason::FocusLost => {
                let _ = self.session.focus_lost();
            }
            _ => {
                let _ = self.session.cancel();
            }
        }
        let _ = self.session.reset();
        self.clear_resolver();
        self.clear_layout_version();
    }
}

/// The target, pointer origin, and source geometry captured at pointer-down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeInteraction {
    pub hit: HitResult,
    pub origin: Point,
    /// Latest pointer position used only to render a visual preview.
    pub preview_point: Option<Point>,
    pub layout_version: u64,
    pub metrics_revision: u64,
    /// Requested width captured at pointer-down for column/flyout resizing.
    pub start_width: Option<f32>,
    /// Visible floating bounds captured at pointer-down for edge/corner resizing.
    pub start_bounds: Option<Rect>,
    /// Original splitter rectangle, captured before the gesture moves it.
    pub start_splitter_rect: Option<Rect>,
    /// Original flyout rectangle, captured before the gesture moves it.
    pub start_flyout_rect: Option<Rect>,
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

#[derive(Default)]
struct ResizeFrameThrottle {
    latest: Option<Point>,
    scheduled: bool,
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
    resize_frame: ResizeFrameThrottle,
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
            resize_frame: ResizeFrameThrottle::default(),
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

    pub(crate) fn has_active_pointer_interaction(&self) -> bool {
        self.interaction.is_some()
            || matches!(
                self.drag.session().state(),
                DragState::Pressed { .. } | DragState::Dragging { .. }
            )
    }

    /// Begins a drag and remembers the Layout revision that owns its model IDs.
    pub(crate) fn begin_drag_interaction(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        point: Point,
    ) -> Result<(), DragSessionError> {
        self.drag
            .session_mut()
            .pointer_down_with_handle(payload, handle, point)?;
        self.drag.clear_resolver();
        self.drag.layout_version = Some(self.layout.version());
        Ok(())
    }

    /// Cancels an active gesture if its model lineage has changed externally.
    ///
    /// Resize commands issued by the active resize pipeline advance the stored
    /// revision explicitly; any other revision change invalidates the gesture.
    pub(crate) fn ensure_pointer_interaction_current(&mut self) -> bool {
        let active_payload = match self.drag.session().state() {
            DragState::Pressed { payload, .. } | DragState::Dragging { payload, .. } => {
                Some(payload)
            }
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => None,
        };
        let expected_version = self
            .interaction
            .map(|interaction| interaction.layout_version)
            .or_else(|| active_payload.and(self.drag.layout_version));

        let version_is_current = match expected_version {
            Some(version) => version == self.layout.version(),
            None => active_payload.is_none() && self.interaction.is_none(),
        };
        let source_is_current = match active_payload {
            Some(payload) => self.drag_payload_source_exists(payload),
            None => true,
        };

        if !version_is_current || !source_is_current {
            if let Err(error) = self.cancel_pointer_interaction_with_reason(
                DragCancelReason::Explicit,
            ) {
                tracing::warn!(%error, "could not cancel stale dock pointer interaction");
            }
            return false;
        }
        true
    }

    fn drag_payload_source_exists(&self, payload: DragPayload) -> bool {
        match payload {
            DragPayload::Panel { panel } => {
                self.layout.panel_instance(panel).is_some()
                    && self.layout.nodes().any(|(_, node)| {
                        matches!(node, Node::Panel(value) if value.panel_id == panel)
                    })
            }
            DragPayload::Group { group } => {
                matches!(self.layout.node(group), Some(Node::Tabs(_)))
            }
            DragPayload::FloatingGroup { group } => self
                .layout
                .floating_group(group)
                .and_then(|floating| self.layout.dock_area(floating.area))
                .is_some_and(|area| self.layout.node(area.root).is_some()),
        }
    }

    /// Called only after a resize command owned by the active interaction succeeds.
    pub(crate) fn record_resize_layout_version(&mut self) {
        if let Some(interaction) = self.interaction.as_mut() {
            interaction.layout_version = self.layout.version();
        }
    }

    /// Starts tracking a resize from a Core hit-test result.
    pub fn begin_resize_interaction(
        &mut self,
        hit: HitResult,
        origin: Point,
    ) -> Result<(), ResizeInteractionError> {
        if self.has_active_pointer_interaction() {
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
        // Snapshot only the geometry values needed by this gesture. No second
        // Dock tree is created; Layout stays the single model owner.
        let rects = self.geometry().ok().cloned();
        let start_splitter_rect = match hit {
            HitResult::Splitter {
                owner,
                boundary_index,
                axis,
            } => rects.as_ref().and_then(|rects| {
                rects
                    .splitters
                    .iter()
                    .find(|splitter| {
                        splitter.owner == owner
                            && splitter.boundary_index == boundary_index
                            && splitter.axis == axis
                    })
                    .map(|splitter| splitter.rect)
            }),
            _ => None,
        };
        let start_flyout_rect = match hit {
            HitResult::FlyoutResizeEdge { panel, .. } => rects
                .as_ref()
                .and_then(|rects| rects.flyout)
                .filter(|flyout| flyout.panel == panel)
                .map(|flyout| flyout.rect),
            _ => None,
        };
        let start_width = match hit {
            HitResult::Splitter {
                owner,
                boundary_index: OUTER_COLUMN_EDGE_BOUNDARY_INDEX,
                axis: Axis::X,
            } => match self.layout.node(owner) {
                Some(Node::Column(column)) => Some(column.width),
                _ => None,
            },
            HitResult::FlyoutResizeEdge { .. } => {
                start_flyout_rect.map(|rect| rect.size.width)
            }
            _ => None,
        };
        let start_bounds = match hit {
            HitResult::FloatingResizeEdge { group, .. } => rects.as_ref().and_then(|rects| {
                rects
                    .floating_groups
                    .iter()
                    .find(|floating| floating.group == group)
                    .map(|floating| floating.rect)
            }),
            _ => None,
        };
        if matches!(hit, HitResult::Splitter { .. }) && start_splitter_rect.is_none() {
            return Err(ResizeInteractionError::NotResizeHandle);
        }
        if matches!(hit, HitResult::FloatingResizeEdge { .. }) && start_bounds.is_none() {
            return Err(ResizeInteractionError::NotResizeHandle);
        }
        if matches!(hit, HitResult::FlyoutResizeEdge { .. })
            && (start_width.is_none() || start_flyout_rect.is_none())
        {
            return Err(ResizeInteractionError::NotResizeHandle);
        }
        if !self.history.begin_transaction(&self.layout) {
            return Err(ResizeInteractionError::AlreadyActive);
        }
        self.resize_frame.latest = None;
        self.drag.clear_resolver();
        self.interaction = Some(ResizeInteraction {
            hit,
            origin,
            preview_point: None,
            layout_version: self.layout.version(),
            metrics_revision: self.metrics_revision,
            start_width,
            start_bounds,
            start_splitter_rect,
            start_flyout_rect,
        });
        Ok(())
    }

    /// Updates only transient preview state; the Dock Layout remains untouched.
    pub(crate) fn set_resize_preview_point(&mut self, point: Point) -> bool {
        if point.validate().is_err() {
            return false;
        }
        let Some(interaction) = self.interaction.as_mut() else {
            return false;
        };
        if interaction.preview_point == Some(point) {
            return false;
        }
        interaction.preview_point = Some(point);
        true
    }

    /// Stores the latest pointer position and reports whether the caller
    /// should schedule the single frame callback for the active gesture.
    pub(crate) fn queue_resize_frame_point(&mut self, point: Point) -> bool {
        if self.interaction.is_none() {
            return false;
        }
        self.resize_frame.latest = Some(point);
        if self.resize_frame.scheduled {
            false
        } else {
            self.resize_frame.scheduled = true;
            true
        }
    }

    /// Takes the most recent pointer position and frees the frame slot.
    pub(crate) fn take_resize_frame_point(&mut self) -> Option<Point> {
        self.resize_frame.scheduled = false;
        self.resize_frame.latest.take()
    }

    /// Drops a pending pointer position when the gesture is explicitly finished.
    pub(crate) fn clear_resize_frame_point(&mut self) {
        self.resize_frame.latest = None;
    }

    /// Ends the active resize and commits its single history checkpoint.
    pub fn end_resize_interaction(&mut self) -> Option<ResizeInteraction> {
        self.resize_frame.latest = None;
        let interaction = self.interaction.take();
        if interaction.is_some() {
            self.history.commit_transaction(&self.layout);
        }
        self.drag.clear_layout_version();
        interaction
    }

    /// Cancels the current pointer gesture as an explicit adapter/model cancellation.
    pub fn cancel_pointer_interaction(&mut self) -> Result<(), CommandError> {
        self.cancel_pointer_interaction_with_reason(DragCancelReason::Explicit)
    }

    /// Cancels the current pointer gesture because its input surface lost focus.
    pub(crate) fn cancel_pointer_interaction_on_focus_loss(
        &mut self,
    ) -> Result<(), CommandError> {
        self.cancel_pointer_interaction_with_reason(DragCancelReason::FocusLost)
    }

    /// Cancels the current pointer gesture because the user pressed Escape.
    pub(crate) fn cancel_pointer_interaction_on_escape(
        &mut self,
    ) -> Result<(), CommandError> {
        self.cancel_pointer_interaction_with_reason(DragCancelReason::Escape)
    }

    fn cancel_pointer_interaction_with_reason(
        &mut self,
        reason: DragCancelReason,
    ) -> Result<(), CommandError> {
        self.drag.cancel_gesture_with_reason(reason);
        let interaction = self.interaction;

        let cancel_result = if interaction.is_some() {
            self.history
                .cancel_transaction(&mut self.layout, &self.metrics)
        } else {
            Ok(false)
        };

        self.interaction = None;
        self.resize_frame.latest = None;
        self.resize_frame.scheduled = false;
        self.rects = None;
        self.rects_key = None;
        self.drag.clear_resolver();

        if let Err(error) = cancel_result {
            // Never leave an open history transaction after capture is lost.
            self.history.commit_transaction(&self.layout);
            return Err(error);
        }

        // Flyout width is transient session state and is intentionally omitted
        // from persistent history snapshots. Restore its pointer-down width
        // explicitly when a flyout resize is cancelled.
        if let Some(interaction) = interaction
            && let (HitResult::FlyoutResizeEdge { panel, .. }, Some(width)) =
                (interaction.hit, interaction.start_width)
            && let Err(error) = self.layout.apply(
                DockCommand::ResizeFlyout { panel, width },
                &self.metrics,
            )
        {
            tracing::debug!(%error, "could not restore flyout width after pointer cancellation");
        }

        Ok(())
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
