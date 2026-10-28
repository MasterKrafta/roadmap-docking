use std::collections::{HashMap, HashSet};

use dock_core::{
    CommandError, CommandOutcome, ComputeRectsError, DockCommand, DockMetrics, DockSide,
    DragDropAction, DragDropRequest, DragHandle, DragPayload, DragSession as CoreDragSession,
    DragState, DropSource, DropTarget, DropZone, FloatingGroupId, InitialLayoutError, Layout, Node,
    NodeId, PanelId, Point, Profile, Rect, ResizeEdge, ValidationError,
    create_initial_layout,
};
use gpui::ScrollHandle;
use ui::Theme;

use crate::requests::{DragOverlay, ResizeHandle};
use crate::{FrameGeometry, dock_metrics_from_theme};

use super::drag_session::{DragGeometrySnapshot, DragGesture, DragStartError, DragUpdateError};
use super::workspace_model::WorkspaceModel;
use super::resize_session::{ResizeSession, ResizeStartError, ResizeUpdateError, StartGeometry};

#[derive(Debug)]
enum Session {
    Idle,
    Resize(ResizeSession),
    Drag(DragGesture),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DockInitializationError {
    #[error(transparent)]
    InitialLayout(#[from] InitialLayoutError),
    #[error("restored workspace layout is invalid: {errors:?}")]
    InvalidRestoredLayout { errors: Vec<ValidationError> },
    #[error("core initial layout is inconsistent: {reason}")]
    UnexpectedCoreLayout { reason: &'static str },
    #[error("core layout does not contain panel '{key}'")]
    MissingPanelKey { key: String },
    #[error("could not {operation}: {source}")]
    CoreCommand {
        operation: &'static str,
        #[source]
        source: CommandError,
    },
    #[error("constructed core layout is invalid: {errors:?}")]
    InvalidConstructedLayout { errors: Vec<ValidationError> },
    #[error("theme is invalid: {reason}")]
    InvalidTheme { reason: &'static str },
}

/// Coordinates one viewport's interactions with the persistent workspace model.
///
/// Layout and history live in `WorkspaceModel`; scroll handles, pointer sessions,
/// and viewport measurements remain local to this controller. Raising a floating
/// window updates visual stacking without creating an undo checkpoint.
pub(crate) struct DockController {
    workspace: WorkspaceModel,
    scroll_handles: HashMap<NodeId, ScrollHandle>,
    session: Session,
    last_viewport: Option<Rect>,
    #[cfg(test)]
    last_error: Option<CommandError>,
}

impl DockController {
    /// Creates the default layout before exposing the controller to user input.
    ///
    /// Initial placement commands seed the layout and intentionally do not
    /// become undo steps; all subsequent commands use the history facade.
    pub(crate) fn new_initial(theme: &Theme) -> Result<Self, DockInitializationError> {
        theme
            .validate()
            .map_err(|reason| DockInitializationError::InvalidTheme { reason })?;
        let (layout, metrics) = build_initial_layout(theme)?;
        Ok(Self::from_layout_and_metrics(theme, layout, metrics))
    }

    /// Installs a restored layout after validating it against the active theme metrics.
    pub(crate) fn new_with_layout(
        theme: &Theme,
        layout: Layout,
    ) -> Result<Self, DockInitializationError> {
        theme
            .validate()
            .map_err(|reason| DockInitializationError::InvalidTheme { reason })?;
        let metrics = dock_metrics_from_theme(theme);
        layout
            .validate(&metrics)
            .map_err(|errors| DockInitializationError::InvalidRestoredLayout { errors })?;
        Ok(Self::from_layout_and_metrics(theme, layout, metrics))
    }

    fn from_layout_and_metrics(theme: &Theme, layout: Layout, metrics: DockMetrics) -> Self {
        Self {
            workspace: WorkspaceModel::new(theme, layout, metrics),
            scroll_handles: HashMap::new(),
            session: Session::Idle,
            last_viewport: None,
            #[cfg(test)]
            last_error: None,
        }
    }

    /// Applies a layout command, recording it unless it only changes floating z-order.
    pub(crate) fn apply(&mut self, command: DockCommand) -> Result<CommandOutcome, CommandError> {
        // A persistent command must not race an uncommitted preview based on an
        // older layout snapshot. Roll the gesture back before mutating history.
        self.cancel_resize();
        self.cancel_drag();

        if let Some(viewport) = self.last_viewport {
            let mut candidate = self.workspace.layout.clone();
            if candidate.apply(command.clone(), &self.workspace.metrics).is_ok()
                && let Err(error) = FrameGeometry::compute(&candidate, viewport, &self.workspace.metrics)
            {
                tracing::warn!(
                    error = %error,
                    "rejecting dock command because the resulting layout does not fit the current viewport"
                );
                return self.track_command_result(
                    Err(CommandError::InvalidValue {
                        field: "layout",
                        reason: "command does not fit the current viewport",
                    }),
                    "apply",
                );
            }
        }

        let result = if matches!(&command, DockCommand::RaiseFloating { .. }) {
            // Stacking order is interaction state, not a user-editable layout step.
            // Core still performs the command transaction and advances the layout
            // version when z-order changes, invalidating geometry snapshots safely.
            self.workspace.layout.apply(command, &self.workspace.metrics)
        } else {
            self.workspace.history.apply(&mut self.workspace.layout, command, &self.workspace.metrics)
        };
        self.track_command_result(result, "apply")
    }

    /// Closes every tab group contained in a floating window.
    ///
    /// Core has no atomic CloseFloating command, so every group is closed through
    /// the standard history path and can be restored with one Undo per group.
    pub(crate) fn close_floating(&mut self, group: FloatingGroupId) -> Result<(), CommandError> {
        let commands = super::commands::close_floating(&self.workspace.layout, group).ok_or_else(|| {
            if self.workspace.layout.floating_group(group).is_some() {
                CommandError::InvalidTarget {
                    reason: "floating group contains an unsupported node type",
                }
            } else {
                CommandError::MissingFloatingGroup { group }
            }
        })?;

        for command in commands {
            self.apply(command)?;
        }
        Ok(())
    }

    /// Restores the preceding recorded layout revision, if one exists.
    pub(crate) fn undo(&mut self) -> Result<bool, CommandError> {
        self.cancel_resize();
        self.cancel_drag();
        let result = self.workspace.history.undo(&mut self.workspace.layout, &self.workspace.metrics);
        self.track_command_result(result, "undo")
    }

    /// Reapplies the next recorded layout revision, if one exists.
    pub(crate) fn redo(&mut self) -> Result<bool, CommandError> {
        self.cancel_resize();
        self.cancel_drag();
        let result = self.workspace.history.redo(&mut self.workspace.layout, &self.workspace.metrics);
        self.track_command_result(result, "redo")
    }

    /// Records a potential drag on pointer-down without starting a Core drag yet.
    ///
    /// The Core state machine starts only after pointer movement crosses its threshold,
    /// preserving ordinary click behavior for tabs.
    pub(crate) fn begin_drag(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        pointer_start: Point,
    ) -> Result<(), DragStartError> {
        if !matches!(&self.session, Session::Idle) {
            return Err(DragStartError::SessionAlreadyActive);
        }

        self.session = Session::Drag(DragGesture::begin(payload, handle, pointer_start)?);
        Ok(())
    }

    /// Clears a pending press or an active drag without changing the layout.
    pub(crate) fn cancel_drag(&mut self) -> bool {
        let Session::Drag(gesture) = &self.session else {
            return false;
        };
        tracing::debug!(
            payload = ?gesture.payload,
            handle = ?gesture.handle,
            origin = ?gesture.origin,
            core_started = gesture.core.is_some(),
            "cancelled dock drag gesture"
        );
        self.session = Session::Idle;
        true
    }

    /// Reports whether this pointer move crosses the threshold and needs a fresh frame.
    pub(crate) fn drag_needs_start(&self, pointer: Point) -> bool {
        match &self.session {
            Session::Drag(gesture) if gesture.core.is_none() => {
                DragGesture::threshold_reached(gesture.origin, pointer, self.workspace.metrics.drag_threshold)
            }
            Session::Idle | Session::Resize(_) | Session::Drag(_) => false,
        }
    }

    /// Starts Core drag tracking after the threshold, then reuses one resolver for the gesture.
    ///
    /// The frame is required only when a pending press first crosses the threshold. Later
    /// movements use the already prepared resolver and update the preview target in place.
    pub(crate) fn update_drag(
        &mut self,
        pointer: Point,
        viewport: Rect,
        frame: Option<&FrameGeometry>,
    ) -> Result<bool, DragUpdateError> {
        // Once a Core drag owns a geometry snapshot, any mismatch invalidates
        // its resolver. Cancel here so callers cannot accidentally keep using
        // stale zones after handling the returned error.
        let stale_snapshot = match &self.session {
            Session::Drag(gesture) if gesture.core.is_some() => {
                gesture.snapshot.is_none_or(|snapshot| {
                    !snapshot.matches(self.workspace.layout.version(), viewport, self.workspace.metrics)
                })
            }
            Session::Idle | Session::Resize(_) | Session::Drag(_) => false,
        };
        if stale_snapshot {
            self.cancel_drag();
            return Err(DragUpdateError::StaleFrameGeometry);
        }

        let start = match &self.session {
            Session::Drag(gesture) if gesture.core.is_none() => {
                if !DragGesture::threshold_reached(
                    gesture.origin,
                    pointer,
                    self.workspace.metrics.drag_threshold,
                ) {
                    return Ok(false);
                }
                Some((gesture.payload, gesture.handle, gesture.origin))
            }
            // The active-Core branch was checked against its snapshot above.
            Session::Drag(_) => None,
            Session::Idle | Session::Resize(_) => return Ok(false),
        };

        if let Some((payload, handle, origin)) = start {
            let frame = frame.ok_or(DragUpdateError::MissingFrameGeometry)?;
            if frame.layout_version() != self.workspace.layout.version()
                || frame.rects().viewport != Some(viewport)
            {
                self.cancel_drag();
                return Err(DragUpdateError::StaleFrameGeometry);
            }
            let snapshot = DragGeometrySnapshot {
                layout_version: frame.layout_version(),
                viewport,
                metrics: self.workspace.metrics,
            };

            let mut core = CoreDragSession::new(&self.workspace.metrics)?;
            core.pointer_down_with_handle(payload, handle, origin)?;
            core.pointer_move(pointer)?;
            if !matches!(core.state(), DragState::Dragging { .. }) {
                return Ok(false);
            }

            // FloatingTitle moves the existing window and must not resolve dock targets.
            // FloatingCapture instead resolves docking zones for the full floating root.
            let mut capture_root_nodes = None;
            let resolver = match (payload, handle) {
                (DragPayload::Panel { panel }, DragHandle::PanelTab | DragHandle::FlyoutPanel) => {
                    Some(frame.prepare_drop_resolver(
                        &self.workspace.layout,
                        DropSource::Panel { panel },
                        &self.workspace.metrics,
                    )?)
                }
                (DragPayload::Group { group }, DragHandle::GroupGrip) => {
                    Some(frame.prepare_drop_resolver(
                        &self.workspace.layout,
                        DropSource::Group { group },
                        &self.workspace.metrics,
                    )?)
                }
                (DragPayload::FloatingGroup { group }, DragHandle::FloatingCapture) => {
                    let (source, nodes) = floating_capture_source(&self.workspace.layout, group)?;
                    capture_root_nodes = Some(nodes);
                    Some(frame.prepare_drop_resolver(&self.workspace.layout, source, &self.workspace.metrics)?)
                }
                (DragPayload::FloatingGroup { .. }, DragHandle::FloatingTitle) => None,
                _ => None,
            };
            if let Some(resolver) = resolver.as_ref() {
                let target = filter_capture_target(
                    handle,
                    capture_root_nodes.as_ref(),
                    resolver.resolve(pointer)?,
                );
                core.set_drop_target(target)?;
            }
            let overlay = resolver.as_ref().and_then(|resolver| {
                core.preview().map(|preview| DragOverlay {
                    preview,
                    handle,
                    zones: resolver
                        .zones()
                        .iter()
                        .copied()
                        .filter(|zone| {
                            capture_target_is_actionable(
                                handle,
                                capture_root_nodes.as_ref(),
                                zone.target,
                            )
                        })
                        .collect(),
                })
            });

            let Session::Drag(gesture) = &mut self.session else {
                return Ok(false);
            };
            gesture.core = Some(core);
            gesture.snapshot = Some(snapshot);
            gesture.resolver = resolver;
            gesture.capture_root_nodes = capture_root_nodes;
            gesture.overlay = overlay;
            return Ok(true);
        }

        let Session::Drag(gesture) = &mut self.session else {
            return Ok(false);
        };
        let Some(core) = gesture.core.as_mut() else {
            return Ok(false);
        };
        core.pointer_move(pointer)?;
        if let Some(resolver) = gesture.resolver.as_ref() {
            let target = filter_capture_target(
                gesture.handle,
                gesture.capture_root_nodes.as_ref(),
                resolver.resolve(pointer)?,
            );
            core.set_drop_target(target)?;
        }
        if let (Some(overlay), Some(preview)) = (&mut gesture.overlay, core.preview()) {
            overlay.preview = preview;
        }
        Ok(true)
    }

    /// Completes an active Core gesture at the release point and returns its semantic request.
    ///
    /// A press released below the threshold is an ordinary click and produces no request.
    /// The returned request is resolved and committed separately by `commit_drop_request`.
    pub(crate) fn finish_drag(
        &mut self,
        pointer: Point,
        viewport: Rect,
    ) -> Result<Option<DragDropRequest>, DragUpdateError> {
        let session = std::mem::replace(&mut self.session, Session::Idle);
        let mut gesture = match session {
            Session::Drag(gesture) => gesture,
            other => {
                self.session = other;
                return Ok(None);
            }
        };

        let Some(mut core) = gesture.core.take() else {
            return Ok(None);
        };
        let snapshot = gesture
            .snapshot
            .ok_or(DragUpdateError::StaleFrameGeometry)?;
        if !snapshot.matches(self.workspace.layout.version(), viewport, self.workspace.metrics) {
            return Err(DragUpdateError::StaleFrameGeometry);
        }

        // Resolve the exact release point rather than reusing the previous move's target.
        core.pointer_move(pointer)?;
        let target = match gesture.resolver.as_ref() {
            Some(resolver) => filter_capture_target(
                gesture.handle,
                gesture.capture_root_nodes.as_ref(),
                resolver.resolve(pointer)?,
            ),
            None => None,
        };
        if gesture.resolver.is_some() {
            core.set_drop_target(target)?;
        }

        Ok(core.pointer_up(pointer, target)?)
    }

    /// Resolves a released drag through Core and commits its command through history.
    ///
    /// A no-op is intentionally not entered in history. Core remains the single source
    /// of truth for validating drop targets and selecting the semantic DockCommand.
    pub(crate) fn commit_drop_request(
        &mut self,
        mut request: DragDropRequest,
        viewport: Rect,
    ) -> Result<DragDropAction, CommandError> {
        let action = match self.resolve_drop_request(&mut request, viewport) {
            Ok(action) => action,
            Err(error) => return self.track_command_result(Err(error), "drop_resolve"),
        };

        let action = match self.apply_floating_move_policy(action, viewport) {
            Ok(action) => action,
            Err(error) => {
                return self.track_command_result(Err(error), "floating_move_policy");
            }
        };

        match &action {
            DragDropAction::Command(command) => {
                // Use the one normal history entry point; Layout::apply is
                // transactional, and history records only committed revisions.
                self.apply(command.clone())?;
            }
            DragDropAction::NoOp => {
                // A rejected previous operation must not remain visible after a
                // successfully resolved no-op drop.
                #[cfg(test)]
                {
                    self.last_error = None;
                }
            }
        }

        Ok(action)
    }

    /// Adds explicit bounds for free-area drops before asking Core to resolve the request.
    fn resolve_drop_request(
        &self,
        request: &mut DragDropRequest,
        viewport: Rect,
    ) -> Result<DragDropAction, CommandError> {
        if request.target == Some(DropTarget::FreeArea) {
            let frame =
                FrameGeometry::compute(&self.workspace.layout, viewport, &self.workspace.metrics).map_err(|_| {
                    CommandError::InvalidTarget {
                        reason: "could not compute source geometry for floating drop",
                    }
                })?;
            super::drop_commit::supply_floating_bounds(
                &self.workspace.layout,
                frame.rects(),
                &self.workspace.metrics,
                viewport,
                request,
            )?;
        }

        super::drop_commit::resolve_drop(&self.workspace.layout, request, &self.workspace.metrics)
    }

    /// Clamps floating-window moves against the currently visible rectangle.
    fn apply_floating_move_policy(
        &self,
        action: DragDropAction,
        viewport: Rect,
    ) -> Result<DragDropAction, CommandError> {
        match action {
            DragDropAction::Command(DockCommand::MoveFloating { group, origin }) => {
                let stored_origin = self
                    .workspace
                    .layout
                    .floating_group(group)
                    .ok_or(CommandError::MissingFloatingGroup { group })?
                    .bounds
                    .origin;
                let frame = FrameGeometry::compute(&self.workspace.layout, viewport, &self.workspace.metrics).map_err(
                    |_| CommandError::InvalidTarget {
                        reason: "could not compute current floating geometry for move",
                    },
                )?;
                let visible_bounds = frame
                    .rects()
                    .floating_groups
                    .iter()
                    .find(|floating| floating.group == group)
                    .map(|floating| floating.rect)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "floating window has no visible geometry in the current viewport",
                    })?;

                // Keep the pointer's absolute displacement relative to the
                // displayed rectangle if older saved coordinates were offscreen.
                let delta = Point::new(origin.x - stored_origin.x, origin.y - stored_origin.y);
                let requested_origin = Point::new(
                    visible_bounds.origin.x + delta.x,
                    visible_bounds.origin.y + delta.y,
                );
                let origin = super::floating_policy::clamp_floating_origin(
                    requested_origin,
                    visible_bounds,
                    viewport,
                )?;
                Ok(DragDropAction::Command(DockCommand::MoveFloating {
                    group,
                    origin,
                }))
            }
            other => Ok(other),
        }
    }

    /// Returns the current drag overlay for rendering, if this is a docking drag.
    pub(crate) fn drag_overlay(&self) -> Option<&DragOverlay> {
        match &self.session {
            Session::Drag(gesture) => gesture.overlay.as_ref(),
            Session::Idle | Session::Resize(_) => None,
        }
    }

    /// Begins a resize only when no other pointer session is active.
    ///
    /// The frame is the normalized geometry snapshot that produced the clicked
    /// handle, so the session captures its starting measurements exactly once.
    pub(crate) fn begin_resize(
        &mut self,
        handle: ResizeHandle,
        pointer_start: Point,
        frame: &FrameGeometry,
    ) -> Result<(), ResizeStartError> {
        if !matches!(&self.session, Session::Idle) {
            return Err(ResizeStartError::SessionAlreadyActive);
        }
        if frame.layout_version() != self.workspace.layout.version() {
            return Err(ResizeStartError::StaleFrameGeometry);
        }

        let session = ResizeSession::begin(
            &self.workspace.layout,
            frame.rects(),
            &self.workspace.metrics,
            handle,
            pointer_start,
        )?;
        self.session = Session::Resize(session);
        Ok(())
    }

    /// Returns the active resize snapshot, if a resize gesture is in progress.
    pub(crate) fn resize_session(&self) -> Option<&ResizeSession> {
        match &self.session {
            Session::Resize(session) => Some(session),
            Session::Idle | Session::Drag(_) => None,
        }
    }

    /// Applies one preview command against the immutable pointer-down snapshot.
    ///
    /// Preview mutations bypass history. A rejected candidate is discarded, so
    /// the previous valid preview remains visible and a later move may recover.
    pub(crate) fn update_resize(
        &mut self,
        pointer: Point,
    ) -> Result<CommandOutcome, ResizeUpdateError> {
        let mut command_error = None;
        let prepared = match &self.session {
            Session::Idle | Session::Drag(_) => Err(ResizeUpdateError::NoActiveSession),
            Session::Resize(session) => match session.command_for_pointer(pointer) {
                Err(error) => Err(error),
                Ok(command) => {
                    let command = preserve_floating_resize_anchor(session, command, &self.workspace.metrics);
                    if let Some(policy) = self.workspace.rail_policy {
                        match policy.preview(
                            session.baseline(),
                            &self.workspace.layout,
                            &command,
                            &self.workspace.metrics,
                        ) {
                            Ok((candidate, _, outcome)) => Ok((candidate, outcome)),
                            Err(error) => {
                                command_error = Some(error);
                                Err(ResizeUpdateError::Command)
                            }
                        }
                    } else {
                        let mut candidate = session.baseline().clone();
                        match candidate.apply(command, &self.workspace.metrics) {
                            Ok(outcome) => Ok((candidate, outcome)),
                            Err(error) => {
                                command_error = Some(error);
                                Err(ResizeUpdateError::Command)
                            }
                        }
                    }
                }
            },
        };

        let (candidate, outcome) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(command_error) = command_error {
                    tracing::warn!(error = %command_error, "dock resize preview was rejected");
                }
                return Err(error);
            }
        };

        self.workspace.layout = candidate;
        Ok(outcome)
    }

    /// Commits the final pointer position as a single persistent history entry.
    ///
    /// The preview is discarded first, then the final absolute-delta command is
    /// applied through UndoRedoHistory against the original pointer-down state.
    /// Rail-mode selection still uses the last preview mode so releasing inside the
    /// hysteresis gap preserves the mode reached during this gesture.
    pub(crate) fn commit_resize(
        &mut self,
        pointer: Point,
    ) -> Result<CommandOutcome, ResizeUpdateError> {
        if !matches!(&self.session, Session::Resize(_)) {
            return Err(ResizeUpdateError::NoActiveSession);
        }
        let session = match std::mem::replace(&mut self.session, Session::Idle) {
            Session::Resize(session) => session,
            Session::Idle => return Err(ResizeUpdateError::NoActiveSession),
            Session::Drag(gesture) => {
                self.session = Session::Drag(gesture);
                return Err(ResizeUpdateError::NoActiveSession);
            }
        };
        let commands = session
            .command_for_pointer(pointer)
            .map(|command| preserve_floating_resize_anchor(&session, command, &self.workspace.metrics))
            .and_then(|command| {
                if let Some(policy) = self.workspace.rail_policy {
                    match policy.preview(session.baseline(), &self.workspace.layout, &command, &self.workspace.metrics)
                    {
                        Ok((_, commands, _)) => Ok(commands),
                        Err(_) => Err(ResizeUpdateError::Command),
                    }
                } else {
                    Ok(vec![command])
                }
            });
        self.workspace.layout = session.into_baseline();

        let commands = commands?;
        let result = self
            .workspace
            .history
            .apply_sequence(&mut self.workspace.layout, commands, &self.workspace.metrics)
            .and_then(|mut outcomes| {
                outcomes.pop().ok_or(CommandError::InvalidTarget {
                    reason: "resize command sequence returned no outcome",
                })
            });
        self.track_command_result(result, "resize_commit")
            .map_err(|_| ResizeUpdateError::Command)
    }

    /// Cancels an active resize and restores the exact pointer-down snapshot.
    ///
    /// Returns false when there is no resize session to cancel.
    pub(crate) fn cancel_resize(&mut self) -> bool {
        if !matches!(&self.session, Session::Resize(_)) {
            return false;
        }
        let session = match std::mem::replace(&mut self.session, Session::Idle) {
            Session::Resize(session) => session,
            Session::Idle => return false,
            Session::Drag(gesture) => {
                self.session = Session::Drag(gesture);
                return false;
            }
        };
        self.workspace.layout = session.into_baseline();
        true
    }

    /// Computes frame geometry and synchronizes the measured canvas remainder.
    ///
    /// The viewport is host-measured. Once geometry resolves the dock columns,
    /// record the resulting canvas bounds so subsequent column-resize commands
    /// use the current window size rather than a stale saved measurement.
    pub(crate) fn frame(&mut self, viewport: Rect) -> Result<FrameGeometry, ComputeRectsError> {
        self.last_viewport = Some(viewport);
        let frame = match FrameGeometry::compute(&self.workspace.layout, viewport, &self.workspace.metrics) {
            Ok(frame) => frame,
            Err(error @ ComputeRectsError::InsufficientCanvasWidth { .. }) => {
                self.recover_main_columns_for_viewport(viewport, error)?
            }
            Err(error) => return Err(error),
        };
        self.workspace.layout
            .update_canvas_bounds(frame.rects().canvas)
            .map_err(ComputeRectsError::Geometry)?;

        // Keep handles for live columns even when the viewport currently has no overflow,
        // but discard handles whose column was removed from the layout.
        let live_columns: HashSet<NodeId> = self
            .workspace
            .layout
            .nodes()
            .filter_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
            .collect();
        self.scroll_handles
            .retain(|column_id, _| live_columns.contains(column_id));

        Ok(frame)
    }

    /// Collapses main-dock columns into icon rails when a restored layout is too wide.
    ///
    /// This preserves every panel and tries outer columns first, keeping the
    /// canvas usable instead of replacing the complete dock surface with an error.
    fn recover_main_columns_for_viewport(
        &mut self,
        viewport: Rect,
        original_error: ComputeRectsError,
    ) -> Result<FrameGeometry, ComputeRectsError> {
        if self.workspace.layout.profile() != Profile::Photoshop {
            return Err(original_error);
        }

        let left_columns = main_column_ids_for_viewport_recovery(&self.workspace.layout, DockSide::Left);
        let right_columns = main_column_ids_for_viewport_recovery(&self.workspace.layout, DockSide::Right);
        let mut collapse_order = Vec::new();

        // Preserve the columns nearest the canvas while shrinking outer columns first.
        collapse_order.extend(
            left_columns
                .iter()
                .take(left_columns.len().saturating_sub(1))
                .copied(),
        );
        collapse_order.extend(
            right_columns
                .iter()
                .rev()
                .take(right_columns.len().saturating_sub(1))
                .copied(),
        );
        collapse_order.extend(left_columns.last().copied());
        collapse_order.extend(right_columns.first().copied());

        let mut candidate = self.workspace.layout.clone();
        let mut collapsed_columns = Vec::new();
        for column_id in collapse_order {
            let Some(Node::Column(column)) = candidate.node(column_id) else {
                continue;
            };
            if column.mode == dock_core::ColumnMode::IconsOnly {
                continue;
            }

            if candidate
                .apply(
                    DockCommand::SetColumnMode {
                        column: column_id,
                        mode: dock_core::ColumnMode::IconsOnly,
                    },
                    &self.workspace.metrics,
                )
                .is_err()
            {
                continue;
            }
            collapsed_columns.push(column_id);

            if let Ok(frame) = FrameGeometry::compute(&candidate, viewport, &self.workspace.metrics) {
                self.workspace.layout = candidate;
                // Viewport recovery is an automatic presentation change, not a
                // user action that should leave undo snapshots which no longer fit.
                self.workspace.history.clear();
                tracing::warn!(
                    columns = ?collapsed_columns,
                    viewport_width = viewport.size.width,
                    cause = %original_error,
                    "collapsed main dock columns to icon rails to restore a usable viewport"
                );
                return Ok(frame);
            }
        }

        Err(original_error)
    }

    fn track_command_result<T>(
        &mut self,
        result: Result<T, CommandError>,
        operation: &'static str,
    ) -> Result<T, CommandError> {
        match result {
            Ok(value) => {
                #[cfg(test)]
                {
                    self.last_error = None;
                }
                Ok(value)
            }
            Err(error) => {
                tracing::warn!(
                    operation,
                    error = %error,
                    "dock controller operation failed"
                );
                #[cfg(test)]
                {
                    self.last_error = Some(error.clone());
                }
                Err(error)
            }
        }
    }

    /// Exposes the most recent command error to controller tests.
    #[cfg(test)]
    pub(crate) fn last_error(&self) -> Option<&CommandError> {
        self.last_error.as_ref()
    }

    pub(crate) fn layout(&self) -> &Layout {
        &self.workspace.layout
    }

    pub(crate) fn metrics(&self) -> &DockMetrics {
        &self.workspace.metrics
    }

    pub(crate) fn scroll_handles(&self) -> &HashMap<NodeId, ScrollHandle> {
        &self.scroll_handles
    }

    pub(crate) fn scroll_handles_mut(&mut self) -> &mut HashMap<NodeId, ScrollHandle> {
        &mut self.scroll_handles
    }
}

fn main_column_ids_for_viewport_recovery(layout: &Layout, side: DockSide) -> Vec<NodeId> {
    let Some(area) = layout.main_area(side) else {
        return Vec::new();
    };
    match layout.node(area.root) {
        Some(Node::Column(_)) => vec![area.root],
        Some(Node::Split(split)) if split.axis == dock_core::Axis::X => split
            .children
            .iter()
            .copied()
            .filter(|id| matches!(layout.node(*id), Some(Node::Column(_))))
            .collect(),
        _ => Vec::new(),
    }
}

fn floating_capture_source(
    layout: &Layout,
    group: dock_core::FloatingGroupId,
) -> Result<(DropSource, HashSet<NodeId>), ComputeRectsError> {
    let root = layout
        .floating_group(group)
        .and_then(|floating| layout.dock_area(floating.area))
        .map(|area| area.root)
        .ok_or(ComputeRectsError::InvalidLayoutState)?;

    let source = match layout.node(root) {
        Some(Node::Tabs(_)) => DropSource::Group { group: root },
        Some(Node::Panel(panel)) => DropSource::Panel {
            panel: panel.panel_id,
        },
        // In the active Photoshop profile, Core's DockFloating mapping cannot
        // re-dock these composite roots through this target pipeline. The UI hides capture.
        Some(Node::Column(_) | Node::Split(_)) | None => {
            return Err(ComputeRectsError::InvalidLayoutState);
        }
    };

    Ok((source, crate::tree_query::subtree_nodes(layout, root)))
}

fn capture_target_is_actionable(
    handle: DragHandle,
    capture_root_nodes: Option<&HashSet<NodeId>>,
    target: DropTarget,
) -> bool {
    if handle != DragHandle::FloatingCapture {
        return true;
    }
    if target == DropTarget::FreeArea {
        return false;
    }

    !crate::tree_query::drop_target_node(target)
        .is_some_and(|node| capture_root_nodes.is_some_and(|nodes| nodes.contains(&node)))
}

fn filter_capture_target(
    handle: DragHandle,
    capture_root_nodes: Option<&HashSet<NodeId>>,
    target: Option<DropTarget>,
) -> Option<DropTarget> {
    target.filter(|target| capture_target_is_actionable(handle, capture_root_nodes, *target))
}

/// Builds the starting arrangement inside the controller so no other module
/// mutates a live Layout. The setup commands are deliberately outside history.
fn build_initial_layout(theme: &Theme) -> Result<(Layout, DockMetrics), DockInitializationError> {
    let metrics = dock_metrics_from_theme(theme);
    let descriptors = crate::panels::descriptors(theme);
    let mut layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)?;

    let (single_tab_group, multi_tab_group) = {
        let area = layout.main_area(DockSide::Right).ok_or(
            DockInitializationError::UnexpectedCoreLayout {
                reason: "factory did not create a right dock area",
            },
        )?;
        let Some(Node::Column(column)) = layout.node(area.root) else {
            return Err(DockInitializationError::UnexpectedCoreLayout {
                reason: "factory right dock root is not a Column",
            });
        };
        if column.groups.len() != 2 {
            return Err(DockInitializationError::UnexpectedCoreLayout {
                reason: "factory did not create the expected two tab groups",
            });
        }
        (column.groups[0], column.groups[1])
    };

    let color_panel = panel_id_for_key(&layout, "color")?;
    let history_panel = panel_id_for_key(&layout, "history")?;

    layout
        .apply(
            DockCommand::MovePanel {
                panel: color_panel,
                target: DropTarget::Tabs {
                    group: multi_tab_group,
                    index: 0,
                },
            },
            &metrics,
        )
        .map_err(|source| DockInitializationError::CoreCommand {
            operation: "place the second tab group",
            source,
        })?;

    // Move history to a different group first. Dock-core intentionally treats
    // moving a panel to the edge of its own group as a no-op.
    layout
        .apply(
            DockCommand::MovePanel {
                panel: history_panel,
                target: DropTarget::Tabs {
                    group: single_tab_group,
                    index: 1,
                },
            },
            &metrics,
        )
        .map_err(|source| DockInitializationError::CoreCommand {
            operation: "stage the history panel",
            source,
        })?;

    layout
        .apply(
            DockCommand::MovePanel {
                panel: history_panel,
                target: DropTarget::GroupEdge {
                    group: multi_tab_group,
                    zone: DropZone::Bottom,
                },
            },
            &metrics,
        )
        .map_err(|source| DockInitializationError::CoreCommand {
            operation: "create the lower group",
            source,
        })?;

    layout
        .apply(
            DockCommand::MoveGroupToMainColumn {
                group: single_tab_group,
                side: DockSide::Right,
                index: 0,
            },
            &metrics,
        )
        .map_err(|source| DockInitializationError::CoreCommand {
            operation: "create the adjacent right column",
            source,
        })?;

    let split_column = layout
        .nodes()
        .find_map(|(id, node)| match node {
            Node::Column(column) if column.groups.len() == 2 => Some(id),
            _ => None,
        })
        .ok_or(DockInitializationError::UnexpectedCoreLayout {
            reason: "initial layout did not contain a two-group column",
        })?;
    let upper_group_ratio = theme.metrics.default_column_split_ratio;
    layout
        .apply(
            DockCommand::RestoreGroupRatios {
                column: split_column,
                ratios: vec![upper_group_ratio, 1.0 - upper_group_ratio],
            },
            &metrics,
        )
        .map_err(|source| DockInitializationError::CoreCommand {
            operation: "apply the initial column group split ratio",
            source,
        })?;

    layout
        .validate(&metrics)
        .map_err(|errors| DockInitializationError::InvalidConstructedLayout { errors })?;

    Ok((layout, metrics))
}

fn panel_id_for_key(layout: &Layout, key: &str) -> Result<PanelId, DockInitializationError> {
    layout
        .panel_instances()
        .find_map(|(id, instance)| (instance.key.as_str() == key).then_some(id))
        .ok_or_else(|| DockInitializationError::MissingPanelKey {
            key: key.to_owned(),
        })
}

/// Corrects the origin when Core clamps a floating resize to its minimum size.
///
/// ResizeFloatingBounds keeps the requested origin while clamping dimensions. For
/// left/top edges, that would move the opposite edge during a minimum-size clamp.
fn preserve_floating_resize_anchor(
    session: &ResizeSession,
    command: DockCommand,
    metrics: &DockMetrics,
) -> DockCommand {
    let (group, requested_bounds) = match &command {
        DockCommand::ResizeFloatingBounds { group, bounds } => (*group, *bounds),
        _ => return command,
    };
    let ResizeHandle::FloatingEdge { edge, .. } = session.handle() else {
        return command;
    };
    let StartGeometry::FloatingEdge {
        bounds: start_bounds,
    } = session.start_geometry()
    else {
        return command;
    };
    let adjusts_x = matches!(
        edge,
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
    );
    let adjusts_y = matches!(
        edge,
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
    );
    if !adjusts_x && !adjusts_y {
        return command;
    }

    let mut probe = session.baseline().clone();
    if probe.apply(command.clone(), metrics).is_err() {
        return command;
    }
    let Some(actual_bounds) = probe.floating_group(group).map(|floating| floating.bounds) else {
        return command;
    };

    let mut corrected_bounds = actual_bounds;
    if adjusts_x && actual_bounds.size.width > requested_bounds.size.width {
        corrected_bounds.origin.x =
            start_bounds.origin.x + start_bounds.size.width - actual_bounds.size.width;
    }
    if adjusts_y && actual_bounds.size.height > requested_bounds.size.height {
        corrected_bounds.origin.y =
            start_bounds.origin.y + start_bounds.size.height - actual_bounds.size.height;
    }

    if corrected_bounds == actual_bounds {
        command
    } else {
        DockCommand::ResizeFloatingBounds {
            group,
            bounds: corrected_bounds,
        }
    }
}

#[cfg(test)]
mod floating_resize_anchor_tests {
    use super::DockController;
    use crate::requests::ResizeHandle;
    use dock_core::{DockCommand, Node, Point, Rect, ResizeEdge, Size};
    use ui::Theme;

    fn assert_opposite_edge_anchored(edge: ResizeEdge, original: Rect, actual: Rect) {
        match edge {
            ResizeEdge::Left => {
                let original_right = original.origin.x + original.size.width;
                let actual_right = actual.origin.x + actual.size.width;
                assert!((actual_right - original_right).abs() < 0.01);
                assert!(
                    actual.size.width > 1.0,
                    "Core should clamp to minimum width"
                );
            }
            ResizeEdge::Top => {
                let original_bottom = original.origin.y + original.size.height;
                let actual_bottom = actual.origin.y + actual.size.height;
                assert!((actual_bottom - original_bottom).abs() < 0.01);
                assert!(
                    actual.size.height > 1.0,
                    "Core should clamp to minimum height"
                );
            }
            ResizeEdge::TopLeft => {
                let original_right = original.origin.x + original.size.width;
                let actual_right = actual.origin.x + actual.size.width;
                let original_bottom = original.origin.y + original.size.height;
                let actual_bottom = actual.origin.y + actual.size.height;
                assert!((actual_right - original_right).abs() < 0.01);
                assert!((actual_bottom - original_bottom).abs() < 0.01);
                assert!(
                    actual.size.width > 1.0,
                    "Core should clamp to minimum width"
                );
                assert!(
                    actual.size.height > 1.0,
                    "Core should clamp to minimum height"
                );
            }
            _ => unreachable!("test covers left, top, and top-left edges"),
        }
    }

    #[test]
    fn floating_left_and_top_resize_keep_opposite_edge_anchored_at_minimum() {
        for edge in [ResizeEdge::Left, ResizeEdge::Top, ResizeEdge::TopLeft] {
            let mut controller = DockController::new_initial(&Theme::default()).unwrap();
            let group = controller
                .layout()
                .nodes()
                .find_map(|(_, node)| match node {
                    Node::Column(column) if column.groups.len() > 1 => {
                        column.groups.first().copied()
                    }
                    _ => None,
                })
                .expect("initial layout should contain a multi-group column");
            let initial_bounds = Rect::new(Point::new(80.0, 60.0), Size::new(500.0, 400.0));
            controller
                .apply(DockCommand::Float {
                    node: group,
                    bounds: initial_bounds,
                })
                .unwrap();
            let (floating_id, original_bounds) = controller
                .layout()
                .floating_groups()
                .next()
                .map(|(id, floating)| (id, floating.bounds))
                .expect("Float should create a floating window");
            let frame = controller
                .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
                .unwrap();

            let start = match edge {
                ResizeEdge::Left => Point::new(
                    original_bounds.origin.x,
                    original_bounds.origin.y + original_bounds.size.height / 2.0,
                ),
                ResizeEdge::Top => Point::new(
                    original_bounds.origin.x + original_bounds.size.width / 2.0,
                    original_bounds.origin.y,
                ),
                ResizeEdge::TopLeft => original_bounds.origin,
                _ => unreachable!("test covers the left, top, and top-left edges"),
            };
            let end = match edge {
                ResizeEdge::Left => Point::new(start.x + original_bounds.size.width - 1.0, start.y),
                ResizeEdge::Top => Point::new(start.x, start.y + original_bounds.size.height - 1.0),
                ResizeEdge::TopLeft => Point::new(
                    start.x + original_bounds.size.width - 1.0,
                    start.y + original_bounds.size.height - 1.0,
                ),
                _ => unreachable!("test covers the left, top, and top-left edges"),
            };
            controller
                .begin_resize(
                    ResizeHandle::FloatingEdge {
                        group: floating_id,
                        edge,
                    },
                    start,
                    &frame,
                )
                .unwrap();
            controller.update_resize(end).unwrap();
            let preview_bounds = controller
                .layout()
                .floating_group(floating_id)
                .unwrap()
                .bounds;
            assert_opposite_edge_anchored(edge, original_bounds, preview_bounds);

            controller.commit_resize(end).unwrap();
            let committed_bounds = controller
                .layout()
                .floating_group(floating_id)
                .unwrap()
                .bounds;
            assert_opposite_edge_anchored(edge, original_bounds, committed_bounds);
            assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
        }
    }
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
