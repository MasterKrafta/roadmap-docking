use std::collections::{HashMap, VecDeque};

use dock_core::{
    Axis, CommandError, CommandValue, DockCommand, DockMetrics, DragCancelReason, DragHandle,
    DragPayload, DragSession, DragSessionError, DragState, DropSource, DropZoneResolver,
    FloatingGroupId, FlyoutState, GeometryError, HitResult, InitialLayoutError, Layout,
    LayoutRects, Node, OUTER_COLUMN_EDGE_BOUNDARY_INDEX, PanelId, PanelKey, Point, Profile,
    Rect, ValidationError, create_initial_layout,
};
use gpui::{AnyView, EventEmitter, WindowId};
use thiserror::Error;

use crate::registry::{PanelRegistry, PanelViewCache};

#[path = "controller_commands.rs"]
mod commands;
#[path = "controller_layout_cache.rs"]
mod layout_cache;

/// Events emitted by the controller entity after successful model operations.
#[derive(Clone, Debug)]
pub enum DockEvent {
    LayoutChanged {
        version: u64,
    },
    PanelOpened {
        panel: PanelId,
        key: PanelKey,
    },
    PanelClosed {
        panel: PanelId,
        key: PanelKey,
    },
    PanelActivated {
        group: dock_core::NodeId,
        panel: PanelId,
    },
    /// Effective values returned by Core after one resize operation.
    ResizeApplied {
        hit: Option<HitResult>,
        actual_values: Vec<CommandValue>,
        version: u64,
    },
    CommandFailed(CommandError),
}

impl EventEmitter<DockEvent> for DockController {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RectCacheKey {
    window_id: Option<WindowId>,
    layout_version: u64,
    viewport: Rect,
    metrics_revision: u64,
    chrome: dock_core::ChromeMode,
    flyout: Option<FlyoutState>,
    /// ResizeFlyout is transient and does not advance Layout::version().
    /// Its remembered width therefore participates in the geometry cache key.
    flyout_width: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FloatingRectCacheKey {
    window_id: WindowId,
    group: FloatingGroupId,
    layout_version: u64,
    metrics_revision: u64,
    chrome: dock_core::ChromeMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ResolverCacheKey {
    Main(RectCacheKey),
    Floating(FloatingRectCacheKey),
}

struct FloatingGeometryCache {
    key: FloatingRectCacheKey,
    rects: LayoutRects,
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

    #[error("the main window bounds are not available for positioning a floating panel")]
    MainWindowBoundsUnavailable,
}

/// State for an active drag gesture.
///
/// DragSession and the optional resolver are transient interaction data only;
/// the layout tree remains exclusively owned by dock_core::Layout.
pub struct DragController {
    session: DragSession,
    resolver: Option<DropZoneResolver>,
    resolver_key: Option<ResolverCacheKey>,
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

    fn set_resolver(&mut self, resolver: DropZoneResolver, key: ResolverCacheKey, source: DropSource) {
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
    main_window_id: Option<WindowId>,
    floating_rects: HashMap<WindowId, FloatingGeometryCache>,
    drag: DragController,
    interaction: Option<ResizeInteraction>,
    resize_frame: ResizeFrameThrottle,
    /// An undocked drag remains one history transaction until its native move ends.
    tearoff_transaction_active: bool,
    /// The native adapter must place restored floating windows on available monitors.
    /// Initial native bounds may come from a caller-provided Layout and need monitor recovery.
    initial_monitor_recovery_pending: bool,
    registry: PanelRegistry,
    /// One persistent AnyView per open runtime PanelId. Transfers and tab
    /// activation only change which host receives a clone; they never replace
    /// the cached entity. Replacing the Layout resets this cache.
    panel_views: PanelViewCache<AnyView>,
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
        let initial_monitor_recovery_pending = layout.floating_groups().next().is_some();

        Ok(Self {
            layout,
            history: dock_core::UndoRedoHistory::new(),
            drag: DragController::new(&metrics)?,
            metrics,
            viewport,
            metrics_revision: 0,
            rects: None,
            rects_key: None,
            main_window_id: None,
            floating_rects: HashMap::new(),
            interaction: None,
            resize_frame: ResizeFrameThrottle::default(),
            tearoff_transaction_active: false,
            initial_monitor_recovery_pending,
            registry,
            panel_views: PanelViewCache::default(),
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

    /// Recreates the profile's initial layout without reading or writing a workspace file.
    pub fn reset_layout(&mut self) -> Result<(), InitialLayoutError> {
        let descriptors = self.registry().descriptors_for_core();
        let layout = create_initial_layout(self.layout().profile(), &descriptors, self.metrics())?;
        self.replace_layout(layout);
        Ok(())
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
            if let Err(error) =
                self.cancel_pointer_interaction_with_reason(DragCancelReason::Explicit)
            {
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
                    && self.layout.nodes().any(
                        |(_, node)| matches!(node, Node::Panel(value) if value.panel_id == panel),
                    )
            }
            DragPayload::Group { group } => {
                matches!(self.layout.node(group), Some(Node::Tabs(_)))
            }
            DragPayload::Column { column } => {
                matches!(self.layout.node(column), Some(Node::Column(_)))
            }
            DragPayload::DockArea { side } => self
                .layout
                .main_area(side)
                .is_some_and(|area| self.layout.node(area.root).is_some()),
            DragPayload::FloatingGroup { group } => self
                .layout
                .floating_group(group)
                .and_then(|floating| self.layout.dock_area(floating.area))
                .is_some_and(|area| self.layout.node(area.root).is_some()),
        }
    }

    /// Returns and clears the initial monitor-recovery flag for caller-provided layouts.
    pub(crate) fn take_initial_monitor_recovery_pending(&mut self) -> bool {
        std::mem::take(&mut self.initial_monitor_recovery_pending)
    }

    /// Applies initial monitor-recovery bounds before the layout enters normal use.
    ///
    /// These are environment repairs, not user edits. The controller is freshly
    /// constructed for this layout, so the correction must not become an Undo step.
    pub(crate) fn apply_initial_monitor_bounds(
        &mut self,
        corrections: &[(FloatingGroupId, Rect)],
    ) -> Result<(), CommandError> {
        if corrections.is_empty() {
            return Ok(());
        }
        if self.has_active_pointer_interaction() || self.tearoff_transaction_active {
            return Err(CommandError::InvalidValue {
                field: "initial_monitor_bounds",
                reason: "cannot repair initial monitor bounds during an active pointer interaction",
            });
        }

        let commands = corrections
            .iter()
            .map(|(group, bounds)| DockCommand::ResizeFloatingBounds {
                group: *group,
                bounds: *bounds,
            })
            .collect::<Vec<_>>();
        let before_panels = self.panel_keys();
        let previous_version = self.layout.version();

        match self
            .history
            .apply_sequence(&mut self.layout, commands, &self.metrics)
        {
            Ok(_) => {
                self.finish_success(before_panels, previous_version, std::iter::empty());
                self.history.clear();
                Ok(())
            }
            Err(error) => {
                self.finish_failure(&error);
                Err(error)
            }
        }
    }

    /// Opens one history transaction for a native tear-off / floating-window move.
    pub(crate) fn begin_tearoff_transaction(&mut self) -> bool {
        if self.tearoff_transaction_active || !self.history.begin_transaction(&self.layout) {
            return false;
        }
        self.tearoff_transaction_active = true;
        true
    }

    /// Opens a single history transaction for one continuous native OS resize.
    /// The adapter streams ResizeFloatingBounds commands until WM_EXITSIZEMOVE.
    pub(crate) fn begin_native_resize_transaction(&mut self) -> bool {
        if self.tearoff_transaction_active
            || self.interaction.is_some()
            || !self.history.begin_transaction(&self.layout)
        {
            return false;
        }
        true
    }

    /// Commits the Core bounds updates collected during one native resize loop.
    pub(crate) fn finish_native_resize_transaction(&mut self) -> bool {
        self.history.commit_transaction(&self.layout)
    }

    /// Continues a whole-group or whole-column tear-off as a floating-composition
    /// capture. The history transaction is deliberately left open.
    pub(crate) fn continue_tearoff_as_floating_capture(
        &mut self,
        group: FloatingGroupId,
    ) -> Result<(), DragSessionError> {
        let (origin, position) = match self.drag.session().state() {
            DragState::Pressed {
                origin, position, ..
            }
            | DragState::Dragging {
                origin, position, ..
            } => (origin, position),
            _ => return Err(DragSessionError::NoActiveGesture),
        };
        self.drag.cancel_gesture();
        self.begin_drag_interaction(
            DragPayload::FloatingGroup { group },
            DragHandle::FloatingCapture,
            origin,
        )?;
        if let Err(error) = self.drag.session_mut().pointer_move(position) {
            self.drag.cancel_gesture();
            return Err(error);
        }
        Ok(())
    }

    /// Continues a native tear-off while preserving the identity of the
    /// individual Panel. The temporary floating window is only its transport
    /// surface; drop resolution must still treat the source as a Panel tab.
    pub(crate) fn continue_tearoff_as_panel_capture(
        &mut self,
        panel: PanelId,
    ) -> Result<(), DragSessionError> {
        let (origin, position) = match self.drag.session().state() {
            DragState::Pressed {
                origin, position, ..
            }
            | DragState::Dragging {
                origin, position, ..
            } => (origin, position),
            _ => return Err(DragSessionError::NoActiveGesture),
        };
        self.drag.cancel_gesture();
        self.begin_drag_interaction(
            DragPayload::Panel { panel },
            DragHandle::PanelTab,
            origin,
        )?;
        if let Err(error) = self.drag.session_mut().pointer_move(position) {
            self.drag.cancel_gesture();
            return Err(error);
        }
        Ok(())
    }

    /// Starts a transaction and Core drag session for an already-floating title bar.
    pub(crate) fn begin_floating_window_move(
        &mut self,
        group: FloatingGroupId,
        point: Point,
    ) -> bool {
        if self.has_active_pointer_interaction() || !self.begin_tearoff_transaction() {
            return false;
        }
        if let Err(error) = self.begin_drag_interaction(
            DragPayload::FloatingGroup { group },
            DragHandle::FloatingCapture,
            point,
        ) {
            tracing::debug!(%error, ?group, "could not begin floating-window title drag");
            let _ = self.history.cancel_transaction(&mut self.layout, &self.metrics);
            self.tearoff_transaction_active = false;
            return false;
        }
        true
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
            HitResult::Splitter { .. } | HitResult::FlyoutResizeEdge { .. }
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
            HitResult::FlyoutResizeEdge { .. } => start_flyout_rect.map(|rect| rect.size.width),
            _ => None,
        };
        if matches!(hit, HitResult::Splitter { .. }) && start_splitter_rect.is_none() {
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
    pub(crate) fn cancel_pointer_interaction_on_focus_loss(&mut self) -> Result<(), CommandError> {
        self.cancel_pointer_interaction_with_reason(DragCancelReason::FocusLost)
    }

    /// Cancels the current pointer gesture because the user pressed Escape.
    pub(crate) fn cancel_pointer_interaction_on_escape(&mut self) -> Result<(), CommandError> {
        self.cancel_pointer_interaction_with_reason(DragCancelReason::Escape)
    }

    fn cancel_pointer_interaction_with_reason(
        &mut self,
        reason: DragCancelReason,
    ) -> Result<(), CommandError> {
        self.drag.cancel_gesture_with_reason(reason);
        let interaction = self.interaction;
        let tearoff_was_active = self.tearoff_transaction_active;
        let previous_version = self.layout.version();

        let cancel_result = if interaction.is_some() || tearoff_was_active {
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
            self.tearoff_transaction_active = false;
            return Err(error);
        }
        if tearoff_was_active {
            self.tearoff_transaction_active = false;
            if self.layout.version() != previous_version {
                self.pending_events.push_back(DockEvent::LayoutChanged {
                    version: self.layout.version(),
                });
            }
        }

        // Flyout width is transient session state and is intentionally omitted
        // from persistent history snapshots. Restore its pointer-down width
        // explicitly when a flyout resize is cancelled.
        if let Some(interaction) = interaction
            && let (HitResult::FlyoutResizeEdge { panel, .. }, Some(width)) =
                (interaction.hit, interaction.start_width)
            && let Err(error) = self
                .layout
                .apply(DockCommand::ResizeFlyout { panel, width }, &self.metrics)
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
