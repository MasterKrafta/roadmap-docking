use std::collections::{HashMap, VecDeque};

use dock_core::{
    Axis, CommandError, DockCommand, DockMetrics, DockSide, DragCancelReason, DragHandle,
    DragPayload, DragSessionError, DragState, FloatingGroupId, HitResult, InitialLayoutError,
    Layout, LayoutRects, Node, NodeId, OUTER_COLUMN_EDGE_BOUNDARY_INDEX, PanelId, Point, Profile,
    Rect, create_initial_layout,
};
use gpui::{AnyView, EventEmitter, WindowId};

use crate::{
    drag_host::{DragHostSession, DragSourceHost},
    registry::{PanelRegistry, PanelViewCache},
};

#[path = "controller_commands.rs"]
mod commands;
#[path = "controller_layout_cache.rs"]
mod layout_cache;


#[path = "dock_event.rs"]
mod dock_event;
#[path = "rect_cache_key.rs"]
mod rect_cache_key;
#[path = "floating_rect_cache_key.rs"]
mod floating_rect_cache_key;
#[path = "resolver_cache_key.rs"]
mod resolver_cache_key;
#[path = "floating_geometry_cache.rs"]
mod floating_geometry_cache;
#[path = "dock_controller_config_error.rs"]
mod dock_controller_config_error;
#[path = "panel_command_error.rs"]
mod panel_command_error;
#[path = "drag_controller.rs"]
mod drag_controller;
#[path = "resize_interaction.rs"]
mod resize_interaction;
#[path = "resize_interaction_error.rs"]
mod resize_interaction_error;
#[path = "resize_frame_throttle.rs"]
mod resize_frame_throttle;

pub use dock_event::DockEvent;
pub use dock_controller_config_error::DockControllerConfigError;
pub use drag_controller::DragController;
pub use panel_command_error::PanelCommandError;
pub use resize_interaction::ResizeInteraction;
pub use resize_interaction_error::ResizeInteractionError;
use floating_geometry_cache::FloatingGeometryCache;
use floating_rect_cache_key::FloatingRectCacheKey;
use rect_cache_key::RectCacheKey;
use resolver_cache_key::ResolverCacheKey;
use resize_frame_throttle::ResizeFrameThrottle;

impl EventEmitter<DockEvent> for DockController {}

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
    /// Current native viewport for each permanent Floating Column HWND.
    floating_column_viewports: HashMap<WindowId, (FloatingGroupId, NodeId, f32, f32)>,
    drag: DragController,
    interaction: Option<ResizeInteraction>,
    resize_frame: ResizeFrameThrottle,
    /// One history transaction remains open while an existing FloatingHost moves natively.
    native_window_move_transaction_active: bool,
    /// A native OS resize loop may be active before its first Core bounds update.
    native_resize_session_active: bool,
    /// A separate native OS resize loop also owns an open Core history transaction.
    native_resize_transaction_active: bool,
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
            floating_column_viewports: HashMap::new(),
            interaction: None,
            resize_frame: ResizeFrameThrottle::default(),
            native_window_move_transaction_active: false,
            native_resize_session_active: false,
            native_resize_transaction_active: false,
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

    /// Replaces the Core layout metrics and invalidates every geometry snapshot.
    ///
    /// Metrics cannot change in the middle of an interaction or native history
    /// transaction: pointer baselines and DragSession constraints were captured
    /// against the previous values. The caller publishes the queued event after
    /// this method returns.
    pub fn set_metrics(
        &mut self,
        metrics: DockMetrics,
    ) -> Result<bool, DockControllerConfigError> {
        metrics
            .validate()
            .map_err(DockControllerConfigError::InvalidMetrics)?;
        if self.metrics == metrics {
            return Ok(false);
        }
        if self.has_active_pointer_interaction()
            || self.native_window_move_transaction_active
            || self.native_resize_session_active
            || self.native_resize_transaction_active
            || self.drag.host_session.is_some()
        {
            return Err(DockControllerConfigError::ActiveInteraction);
        }
        self.layout
            .validate(&metrics)
            .map_err(DockControllerConfigError::InvalidLayout)?;

        let revision = self
            .metrics_revision
            .checked_add(1)
            .ok_or(DockControllerConfigError::MetricsRevisionOverflow)?;
        let drag = DragController::new(&metrics)?;

        self.metrics = metrics;
        self.metrics_revision = revision;
        self.drag = drag;
        self.interaction = None;
        self.resize_frame = ResizeFrameThrottle::default();
        self.rects = None;
        self.rects_key = None;
        self.floating_rects.clear();
        self.floating_column_viewports.clear();
        self.pending_events
            .push_back(DockEvent::MetricsChanged { revision });

        Ok(true)
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

    /// Returns the resize interaction only to the native window that owns it.
    pub(crate) fn resize_interaction_for_window(
        &self,
        window_id: WindowId,
    ) -> Option<ResizeInteraction> {
        self.interaction
            .filter(|interaction| interaction.window_id == window_id)
    }

    /// Returns whether the current content drag can transfer this cached PanelView.
    pub(crate) fn can_begin_panel_view_transfer(&self, panel: PanelId) -> bool {
        self.panel_views.get(panel).is_some()
            && self.drag.can_begin_panel_view_transfer(panel)
    }

    /// Marks the current PanelView as owned by the temporary DragHost surface.
    pub(crate) fn begin_panel_view_transfer(&mut self, panel: PanelId) -> bool {
        self.can_begin_panel_view_transfer(panel)
            && self.drag.begin_panel_view_transfer(panel)
    }

    /// Returns the Group's PanelIds with its active tab first for DragHost presentation.
    pub(crate) fn group_panel_ids(&self, group: NodeId) -> Option<Vec<PanelId>> {
        let Node::Tabs(tabs) = self.layout.node(group)? else {
            return None;
        };
        let active_child = tabs.active_child()?;
        let active_panel = match self.layout.node(active_child)? {
            Node::Panel(panel) => panel.panel_id,
            _ => return None,
        };
        let mut panels = tabs
            .children
            .iter()
            .map(|child| match self.layout.node(*child)? {
                Node::Panel(panel) => Some(panel.panel_id),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let active_index = panels.iter().position(|panel| *panel == active_panel)?;
        panels.swap(0, active_index);
        Some(panels)
    }

    pub(crate) fn can_begin_group_view_transfer(
        &self,
        group: NodeId,
        panels: &[PanelId],
    ) -> bool {
        let Some(group_panels) = self.group_panel_ids(group) else {
            return false;
        };
        !group_panels.is_empty()
            && panels.first() == group_panels.first()
            && panels.len() == group_panels.len()
            && group_panels.iter().all(|panel| panels.contains(panel))
            && panels.iter().all(|panel| self.panel_views.get(*panel).is_some())
            && self
                .drag
                .host_session()
                .is_some_and(|session| session.can_begin_group_view_transfer(group, panels))
    }

    pub(crate) fn begin_group_view_transfer(
        &mut self,
        group: NodeId,
        panels: &[PanelId],
    ) -> bool {
        self.can_begin_group_view_transfer(group, panels)
            && self
                .drag
                .host_session
                .as_mut()
                .is_some_and(|session| session.begin_group_view_transfer(group, panels))
    }

    pub(crate) fn active_dock_area_panel_ids(&self, side: DockSide) -> Option<Vec<PanelId>> {
        fn collect_active_panels(layout: &Layout, node: NodeId, panels: &mut Vec<PanelId>) {
            match layout.node(node) {
                Some(Node::Split(split)) => {
                    for child in &split.children {
                        collect_active_panels(layout, *child, panels);
                    }
                }
                Some(Node::Column(column)) => {
                    for group in &column.groups {
                        collect_active_panels(layout, *group, panels);
                    }
                }
                Some(Node::Tabs(tabs)) => {
                    if let Some(child) = tabs.active_child()
                        && let Some(Node::Panel(panel)) = layout.node(child)
                    {
                        panels.push(panel.panel_id);
                    }
                }
                _ => {}
            }
        }

        let area = self.layout.main_area(side)?;
        let mut panels = Vec::new();
        collect_active_panels(&self.layout, area.root, &mut panels);
        Some(panels)
    }

    pub(crate) fn can_begin_dock_area_view_transfer(
        &self,
        side: DockSide,
        panels: &[PanelId],
    ) -> bool {
        let Some(area) = self.layout.main_area(side) else {
            return false;
        };
        let mut columns = Vec::new();
        fn collect_columns(layout: &Layout, node: NodeId, columns: &mut Vec<NodeId>) {
            match layout.node(node) {
                Some(Node::Split(split)) => {
                    for child in &split.children {
                        collect_columns(layout, *child, columns);
                    }
                }
                Some(Node::Column(_)) => columns.push(node),
                _ => {}
            }
        }
        collect_columns(&self.layout, area.root, &mut columns);

        if self
            .layout
            .flyout()
            .is_some_and(|flyout| columns.contains(&flyout.column))
        {
            return false;
        }

        let Some(active_panels) = self.active_dock_area_panel_ids(side) else {
            return false;
        };
        active_panels == panels
            && panels.iter().all(|panel| self.panel_views.get(*panel).is_some())
            && self.drag.host_session().is_some_and(|session| {
                session.source_host() == DragSourceHost::Main
                    && session.source_window_id().is_some()
                    && session.can_begin_dock_area_view_transfer(side, panels)
            })
    }

    pub(crate) fn begin_dock_area_view_transfer(
        &mut self,
        side: DockSide,
        panels: &[PanelId],
    ) -> bool {
        self.can_begin_dock_area_view_transfer(side, panels)
            && self
                .drag
                .host_session
                .as_mut()
                .is_some_and(|session| session.begin_dock_area_view_transfer(side, panels))
    }

    /// Returns the Group replaced by a DragHost placeholder in this persistent source window.
    pub(crate) fn dragged_group_from(&self, source_window_id: WindowId) -> Option<NodeId> {
        let session = self.drag.host_session()?;
        if session.source_window_id() != Some(source_window_id)
            || session.transport_host().is_none()
            || session.transferred_panels().is_empty()
        {
            return None;
        }
        match session.payload() {
            DragPayload::Group { group } => Some(group),
            _ => None,
        }
    }

    /// Returns the Main DockArea replaced by a DragHost placeholder in this persistent source window.
    pub(crate) fn dragged_dock_area_from(&self, source_window_id: WindowId) -> Option<DockSide> {
        let session = self.drag.host_session()?;
        if session.source_window_id() != Some(source_window_id)
            || session.source_host() != DragSourceHost::Main
            || session.transport_host().is_none()
        {
            return None;
        }
        match session.payload() {
            DragPayload::DockArea { side } => Some(side),
            _ => None,
        }
    }

    /// Returns the original cached AnyView after the source placeholder frame has rendered.
    pub(crate) fn panel_view_for_drag(&self, panel: PanelId) -> Option<AnyView> {
        self.drag
            .panel_view_source_detached(panel)
            .then(|| self.panel_views.get(panel).cloned())
            .flatten()
    }

    /// Returns every PanelView currently transferred from this persistent source host.
    ///
    /// Unlike pending_panel_view_transfers_from, this includes views whose source
    /// placeholder has already rendered. The source tab must remain hidden until
    /// the drag session finishes, not only until the transfer handshake completes.
    pub(crate) fn transferred_panel_views_from(
        &self,
        source_window_id: WindowId,
    ) -> Vec<PanelId> {
        let Some(session) = self.drag.host_session() else {
            return Vec::new();
        };
        if session.source_window_id() != Some(source_window_id) {
            return Vec::new();
        }
        session.transferred_panels().to_vec()
    }

    pub(crate) fn pending_panel_view_transfers_from(
        &self,
        source_window_id: WindowId,
    ) -> Vec<PanelId> {
        let Some(session) = self.drag.host_session() else {
            return Vec::new();
        };
        if session.source_window_id() != Some(source_window_id) {
            return Vec::new();
        }
        session
            .transferred_panels()
            .iter()
            .copied()
            .filter(|panel| !session.panel_view_source_detached(*panel))
            .collect()
    }

    pub(crate) fn confirm_panel_view_source_detached(
        &mut self,
        panel: PanelId,
        source_window_id: WindowId,
    ) -> bool {
        self.drag
            .confirm_panel_view_source_detached(panel, source_window_id)
    }

    /// Ends presentation transfer after DragHost has stopped mounting the view.
    pub(crate) fn finish_panel_view_transfer(&mut self, panel: PanelId) -> bool {
        self.drag.finish_panel_view_transfer(panel)
    }

    pub(crate) fn has_active_pointer_interaction(&self) -> bool {
        self.interaction.is_some()
            || matches!(
                self.drag.session().state(),
                DragState::Pressed { .. } | DragState::Dragging { .. }
            )
    }

    /// Begins the Core pointer state without changing adapter source/transport context.
    ///
    /// Both the persistent source Host and DragHost retain the same semantic
    /// payload; the temporary transport window never replaces the drag source.
    fn begin_core_drag_interaction(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        point: Point,
    ) -> Result<(), DragSessionError> {
        // A host-specific completion/cancellation path may leave a terminal
        // state behind. Re-arm it here so the next physical gesture can start.
        if matches!(
            self.drag.session().state(),
            DragState::Dropped { .. } | DragState::Cancelled { .. }
        ) {
            self.drag.session_mut().reset()?;
        }
        self.drag
            .session_mut()
            .pointer_down_with_handle(payload, handle, point)?;
        self.drag.clear_resolver();
        self.drag.layout_version = Some(self.layout.version());
        Ok(())
    }

    /// Begins a drag that is not transporting dock content, such as a floating title move.
    pub(crate) fn begin_drag_interaction(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        point: Point,
    ) -> Result<(), DragSessionError> {
        self.begin_core_drag_interaction(payload, handle, point)?;
        self.drag.cancel_host_session();
        Ok(())
    }

    /// Begins a content drag while preserving its original semantic source and host.
    pub(crate) fn begin_content_drag_interaction(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        point: Point,
        source_host: DragSourceHost,
        source_window_id: WindowId,
    ) -> Result<(), DragSessionError> {
        self.begin_core_drag_interaction(payload, handle, point)?;
        self.drag.begin_host_session(DragHostSession::new(
            payload,
            handle,
            source_host,
            Some(source_window_id),
        ));
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
            // A Main DockArea remains in its original Layout until DragHost
            // releases and applies the one final drop command.
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

    /// Opens one history transaction while an existing FloatingHost moves natively.
    pub(crate) fn begin_native_window_move_transaction(&mut self) -> bool {
        if self.native_window_move_transaction_active
            || self.native_resize_session_active
            || self.native_resize_transaction_active
            || !self.history.begin_transaction(&self.layout)
        {
            return false;
        }
        self.native_window_move_transaction_active = true;
        true
    }

    /// Marks the OS sizing loop active before its first Core bounds update.
    pub(crate) fn begin_native_resize_session(&mut self) -> bool {
        if self.native_window_move_transaction_active
            || self.native_resize_session_active
            || self.native_resize_transaction_active
            || self.has_active_pointer_interaction()
            || self.drag.host_session.is_some()
        {
            return false;
        }
        self.native_resize_session_active = true;
        true
    }

    /// Releases the metric-update guard after WM_EXITSIZEMOVE has completed.
    pub(crate) fn end_native_resize_session(&mut self) {
        self.native_resize_session_active = false;
    }

    /// Opens a single history transaction for one continuous native OS resize.
    /// The adapter streams ResizeFloatingBounds commands until WM_EXITSIZEMOVE.
    pub(crate) fn begin_native_resize_transaction(&mut self) -> bool {
        if !self.native_resize_session_active
            || self.native_window_move_transaction_active
            || self.native_resize_transaction_active
            || self.interaction.is_some()
            || !self.history.begin_transaction(&self.layout)
        {
            return false;
        }
        self.native_resize_transaction_active = true;
        true
    }

    /// Commits the Core bounds updates collected during one native resize loop.
    pub(crate) fn finish_native_resize_transaction(&mut self) -> bool {
        let committed = self.history.commit_transaction(&self.layout);
        self.native_resize_transaction_active = false;
        committed
    }

    /// Starts a transaction and Core drag session for an already-floating title bar.
    pub(crate) fn begin_floating_window_move(
        &mut self,
        group: FloatingGroupId,
        point: Point,
    ) -> bool {
        if self.has_active_pointer_interaction() || !self.begin_native_window_move_transaction() {
            return false;
        }
        if let Err(error) = self.begin_drag_interaction(
            DragPayload::FloatingGroup { group },
            DragHandle::FloatingCapture,
            point,
        ) {
            tracing::debug!(%error, ?group, "could not begin floating-window title drag");
            let _ = self.history.cancel_transaction(&mut self.layout, &self.metrics);
            self.native_window_move_transaction_active = false;
            return false;
        }
        true
    }

    /// Starts a native GroupGrip move without creating a DragHost or transferring PanelViews.
    ///
    /// The persistent Group HWND remains the presentation host for the entire gesture.
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
        let window_id = self
            .main_window_id
            .ok_or(ResizeInteractionError::WindowUnavailable)?;
        self.begin_resize_interaction_with_rects(window_id, hit, origin, None)
    }

    /// Starts a resize using the geometry produced by the host that owns the
    /// pointer. Every host supplies its WindowId and, for Floating, its local
    /// geometry snapshot; Main uses the controller's standard Layout geometry.
    pub(crate) fn begin_resize_interaction_with_rects(
        &mut self,
        window_id: WindowId,
        hit: HitResult,
        origin: Point,
        rects: Option<&LayoutRects>,
    ) -> Result<(), ResizeInteractionError> {
        self.begin_resize_interaction_in_context(window_id, hit, origin, rects, None)
    }

    pub(crate) fn begin_floating_column_resize_interaction_with_rects(
        &mut self,
        window_id: WindowId,
        hit: HitResult,
        origin: Point,
        floating_group: FloatingGroupId,
        column: NodeId,
        rects: Option<&LayoutRects>,
    ) -> Result<(), ResizeInteractionError> {
        if matches!(&hit, HitResult::Splitter { axis: Axis::Y, .. }) {
            let column_height = rects
                .and_then(|rects| {
                    rects
                        .columns
                        .iter()
                        .find(|rect| rect.column == column)
                        .map(|rect| rect.rect.size.height)
                })
                .filter(|height| height.is_finite() && *height > 0.0)
                .ok_or(ResizeInteractionError::NotResizeHandle)?;

            // Persist the current native viewport as transient Core state before
            // opening the history transaction. Undo then restores the true height
            // that the user had before dragging this Group splitter.
            self.history
                .apply(
                    &mut self.layout,
                    DockCommand::SetFloatingColumnHeight {
                        floating_group,
                        column,
                        height: column_height,
                    },
                    &self.metrics,
                )
                .map_err(|error| {
                    tracing::debug!(%error, ?column, "could not synchronize Floating Column height before resize");
                    ResizeInteractionError::NotResizeHandle
                })?;

            let origin_y_offset = self
                .floating_column_viewports
                .get(&window_id)
                .filter(|(group, current_column, _, _)| {
                    *group == floating_group && *current_column == column
                })
                .map(|(_, _, _, origin)| *origin)
                .unwrap_or(0.0);
            self.set_floating_column_viewport(
                window_id,
                floating_group,
                column,
                column_height,
                origin_y_offset,
            );
        }

        self.begin_resize_interaction_in_context(
            window_id,
            hit,
            origin,
            rects,
            Some((floating_group, column)),
        )
    }

    fn begin_resize_interaction_in_context(
        &mut self,
        window_id: WindowId,
        hit: HitResult,
        origin: Point,
        rects: Option<&LayoutRects>,
        floating_column: Option<(FloatingGroupId, NodeId)>,
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

        let owned_rects = if rects.is_none() {
            self.geometry().ok().cloned()
        } else {
            None
        };
        let rects = rects.or(owned_rects.as_ref());

        let start_splitter_rect = match hit {
            HitResult::Splitter {
                owner,
                boundary_index,
                axis,
            } => rects.and_then(|rects| {
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
            window_id,
            floating_column,
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
    pub(crate) fn set_resize_preview_point(
        &mut self,
        window_id: WindowId,
        point: Point,
    ) -> bool {
        if point.validate().is_err() {
            return false;
        }
        let Some(interaction) = self.interaction.as_mut() else {
            return false;
        };
        if interaction.window_id != window_id || interaction.preview_point == Some(point) {
            return false;
        }
        interaction.preview_point = Some(point);
        true
    }

    /// Stores the latest pointer position only for the window that owns the gesture.
    pub(crate) fn queue_resize_frame_point(
        &mut self,
        window_id: WindowId,
        point: Point,
    ) -> bool {
        if !self
            .interaction
            .is_some_and(|interaction| interaction.window_id == window_id)
        {
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

    /// Takes the most recent pointer position and frees the frame slot for its owner.
    pub(crate) fn take_resize_frame_point(&mut self, window_id: WindowId) -> Option<Point> {
        if !self
            .interaction
            .is_some_and(|interaction| interaction.window_id == window_id)
        {
            return None;
        }
        self.resize_frame.scheduled = false;
        self.resize_frame.latest.take()
    }

    /// Drops a pending pointer position only when called by the owning window.
    pub(crate) fn clear_resize_frame_point(&mut self, window_id: WindowId) {
        if self
            .interaction
            .is_some_and(|interaction| interaction.window_id == window_id)
        {
            self.resize_frame.latest = None;
        }
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
        let native_window_move_was_active = self.native_window_move_transaction_active;
        let previous_version = self.layout.version();

        let cancel_result = if interaction.is_some() || native_window_move_was_active {
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
        self.drag.cancel_host_session();

        if let Err(error) = cancel_result {
            // Never leave an open history transaction after capture is lost.
            self.history.commit_transaction(&self.layout);
            self.native_window_move_transaction_active = false;
            return Err(error);
        }
        if native_window_move_was_active {
            self.native_window_move_transaction_active = false;
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
