use std::collections::{HashMap, HashSet};

use dock_core::{
    create_initial_layout, Axis, CommandError, CommandOutcome, ComputeRectsError, DockCommand,
    DockMetrics, DockSide, DragDropAction, DragDropRequest, DragHandle, DragPayload,
    DragSession as CoreDragSession, DragState,
    DropSource, DropTarget, DropZone, Layout, Node, NodeId, PanelId, Point, Profile, Rect,
    ResizeEdge, UndoRedoHistory,
};
use gpui::ScrollHandle;
use ui::Theme;

use crate::{dock_metrics_from_theme, FrameGeometry};
use crate::requests::{DragOverlay, ResizeHandle};

use super::drag_session::{
    DragGeometrySnapshot, DragGesture, DragStartError, DragUpdateError,
};
use super::resize_session::{ResizeSession, ResizeStartError, ResizeUpdateError, StartGeometry};

#[derive(Debug)]
enum Session {
    Idle,
    Resize(ResizeSession),
    Drag(DragGesture),
}

/// Owns the live layout and the view-local state needed to render it.
///
/// All post-initialization layout commands are applied through the history
/// facade, keeping command execution and undo checkpoints on one path.
pub(crate) struct DockController {
    layout: Layout,
    metrics: DockMetrics,
    history: UndoRedoHistory,
    scroll_handles: HashMap<NodeId, ScrollHandle>,
    session: Session,
    last_error: Option<CommandError>,
}

impl DockController {
    /// Creates the default layout before exposing the controller to user input.
    ///
    /// Initial placement commands seed the layout and intentionally do not
    /// become undo steps; all subsequent commands use the history facade.
    pub(crate) fn new_initial(theme: &Theme) -> Result<Self, String> {
        let (layout, metrics) = build_initial_layout(theme)?;
        Ok(Self {
            layout,
            metrics,
            history: UndoRedoHistory::new(),
            scroll_handles: HashMap::new(),
            session: Session::Idle,
            last_error: None,
        })
    }

    /// Applies a layout command and records persistent changes for undo/redo.
    pub(crate) fn apply(
        &mut self,
        command: DockCommand,
    ) -> Result<CommandOutcome, CommandError> {
        // A persistent command must not race an uncommitted preview based on an
        // older layout snapshot. Roll the gesture back before mutating history.
        self.cancel_resize();
        self.cancel_drag();
        let result = self
            .history
            .apply(&mut self.layout, command, &self.metrics);
        self.track_command_result(result, "apply")
    }

    /// Restores the preceding recorded layout revision, if one exists.
    pub(crate) fn undo(&mut self) -> Result<bool, CommandError> {
        self.cancel_resize();
        self.cancel_drag();
        let result = self.history.undo(&mut self.layout, &self.metrics);
        self.track_command_result(result, "undo")
    }

    /// Reapplies the next recorded layout revision, if one exists.
    pub(crate) fn redo(&mut self) -> Result<bool, CommandError> {
        self.cancel_resize();
        self.cancel_drag();
        let result = self.history.redo(&mut self.layout, &self.metrics);
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
        self.last_error = None;
        true
    }

    /// Reports whether this pointer move crosses the threshold and needs a fresh frame.
    pub(crate) fn drag_needs_start(&self, pointer: Point) -> bool {
        match &self.session {
            Session::Drag(gesture) if gesture.core.is_none() => DragGesture::threshold_reached(
                gesture.origin,
                pointer,
                self.metrics.drag_threshold,
            ),
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
            Session::Drag(gesture) if gesture.core.is_some() => gesture.snapshot.is_none_or(
                |snapshot| !snapshot.matches(self.layout.version(), viewport, self.metrics),
            ),
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
                    self.metrics.drag_threshold,
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
            if frame.layout_version() != self.layout.version()
                || frame.rects().viewport != Some(viewport)
            {
                self.cancel_drag();
                return Err(DragUpdateError::StaleFrameGeometry);
            }
            let snapshot = DragGeometrySnapshot {
                layout_version: frame.layout_version(),
                viewport,
                metrics: self.metrics,
            };

            let mut core = CoreDragSession::new(&self.metrics)?;
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
                        &self.layout,
                        DropSource::Panel { panel },
                        &self.metrics,
                    )?)
                }
                (DragPayload::Group { group }, DragHandle::GroupGrip) => {
                    Some(frame.prepare_drop_resolver(
                        &self.layout,
                        DropSource::Group { group },
                        &self.metrics,
                    )?)
                }
                (DragPayload::FloatingGroup { group }, DragHandle::FloatingCapture) => {
                    let (source, nodes) = floating_capture_source(&self.layout, group)?;
                    capture_root_nodes = Some(nodes);
                    Some(frame.prepare_drop_resolver(&self.layout, source, &self.metrics)?)
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
            self.last_error = None;
            return Ok(None);
        };
        let snapshot = gesture
            .snapshot
            .ok_or(DragUpdateError::StaleFrameGeometry)?;
        if !snapshot.matches(self.layout.version(), viewport, self.metrics) {
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

        self.last_error = None;
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
        // Resolution is read-only. Treat geometry/policy failures like command
        // failures so the controller never leaves a stale last_error behind.
        let resolution = (|| -> Result<DragDropAction, CommandError> {
            if request.target == Some(DropTarget::FreeArea) {
                let frame = FrameGeometry::compute(&self.layout, viewport, &self.metrics)
                    .map_err(|_| CommandError::InvalidTarget {
                        reason: "could not compute source geometry for floating drop",
                    })?;
                super::drop_commit::supply_floating_bounds(
                    &self.layout,
                    frame.rects(),
                    &self.metrics,
                    viewport,
                    &mut request,
                )?;
            }

            super::drop_commit::resolve_drop(&self.layout, &request, &self.metrics)
        })();

        let action = match resolution {
            Ok(action) => action,
            Err(error) => return self.track_command_result(Err(error), "drop_resolve"),
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
                self.last_error = None;
            }
        }

        Ok(action)
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
        if frame.layout_version() != self.layout.version() {
            return Err(ResizeStartError::StaleFrameGeometry);
        }

        let session = ResizeSession::begin(
            &self.layout,
            frame.rects(),
            &self.metrics,
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
        let (mut candidate, command) = match &self.session {
            Session::Idle | Session::Drag(_) => {
                return Err(ResizeUpdateError::NoActiveSession);
            }
            Session::Resize(session) => {
                let command = session.command_for_pointer(pointer)?;
                let command =
                    preserve_floating_resize_anchor(session, command, &self.metrics);
                (session.baseline().clone(), command)
            }
        };

        match candidate.apply(command, &self.metrics) {
            Ok(outcome) => {
                self.layout = candidate;
                self.last_error = None;
                Ok(outcome)
            }
            Err(error) => {
                tracing::warn!(error = %error, "dock resize preview was rejected");
                self.last_error = Some(error.clone());
                Err(ResizeUpdateError::Command)
            }
        }
    }

    /// Commits the final pointer position as a single persistent history entry.
    ///
    /// The preview is discarded first, then the final absolute-delta command is
    /// applied through UndoRedoHistory against the original pointer-down state.
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
        let command = session
            .command_for_pointer(pointer)
            .map(|command| preserve_floating_resize_anchor(&session, command, &self.metrics));
        self.layout = session.into_baseline();
        self.last_error = None;

        let command = command?;
        let result = self.history.apply(&mut self.layout, command, &self.metrics);
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
        self.layout = session.into_baseline();
        self.last_error = None;
        true
    }

    /// Computes frame geometry and synchronizes the measured canvas remainder.
    ///
    /// The viewport is host-measured. Once geometry resolves the dock columns,
    /// record the resulting canvas bounds so subsequent column-resize commands
    /// use the current window size rather than a stale saved measurement.
    pub(crate) fn frame(&mut self, viewport: Rect) -> Result<FrameGeometry, ComputeRectsError> {
        let frame = FrameGeometry::compute(&self.layout, viewport, &self.metrics)?;
        self.layout
            .update_canvas_bounds(frame.rects().canvas)
            .map_err(ComputeRectsError::Geometry)?;
        Ok(frame)
    }

    /// Returns the most recent command/history error, if the last operation failed.
    pub(crate) fn last_error(&self) -> Option<&CommandError> {
        self.last_error.as_ref()
    }

    fn track_command_result<T>(
        &mut self,
        result: Result<T, CommandError>,
        operation: &'static str,
    ) -> Result<T, CommandError> {
        match result {
            Ok(value) => {
                self.last_error = None;
                Ok(value)
            }
            Err(error) => {
                tracing::warn!(
                    operation,
                    error = %error,
                    "dock controller operation failed"
                );
                self.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn layout(&self) -> &Layout {
        &self.layout
    }

    pub(crate) fn metrics(&self) -> &DockMetrics {
        &self.metrics
    }

    pub(crate) fn scroll_handles(&self) -> &HashMap<NodeId, ScrollHandle> {
        &self.scroll_handles
    }

    pub(crate) fn scroll_handles_mut(&mut self) -> &mut HashMap<NodeId, ScrollHandle> {
        &mut self.scroll_handles
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
        // Core's DockFloating transaction cannot dock Column or Split roots
        // through this target pipeline. The UI hides capture for those shapes.
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
fn build_initial_layout(theme: &Theme) -> Result<(Layout, DockMetrics), String> {
    let metrics = dock_metrics_from_theme(theme);
    let descriptors = crate::panels::descriptors(theme);
    let mut layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .map_err(|error| error.to_string())?;

    let (single_tab_group, multi_tab_group) = {
        let area = layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Core factory did not create a right dock area".to_owned())?;
        let Some(Node::Column(column)) = layout.node(area.root) else {
            return Err("Core factory right dock root is not a Column".to_owned());
        };
        if column.groups.len() != 2 {
            return Err("Core factory did not create the expected two tab groups".to_owned());
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
        .map_err(|error| format!("Could not place the second tab group: {error:?}"))?;

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
        .map_err(|error| format!("Could not stage the history panel: {error:?}"))?;

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
        .map_err(|error| format!("Could not create the lower group: {error:?}"))?;

    layout
        .apply(
            DockCommand::MoveGroupToMainColumn {
                group: single_tab_group,
                side: DockSide::Right,
                index: 0,
            },
            &metrics,
        )
        .map_err(|error| format!("Could not create the adjacent right column: {error:?}"))?;

    layout
        .validate(&metrics)
        .map_err(|errors| format!("Constructed Core layout is invalid: {errors:?}"))?;

    Ok((layout, metrics))
}

fn panel_id_for_key(layout: &Layout, key: &str) -> Result<PanelId, String> {
    layout
        .panel_instances()
        .find_map(|(id, instance)| (instance.key.as_str() == key).then_some(id))
        .ok_or_else(|| format!("Core layout does not contain the panel '{key}'"))
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
    let Some(actual_bounds) = probe
        .floating_group(group)
        .map(|floating| floating.bounds)
    else {
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
                assert!(actual.size.width > 1.0, "Core should clamp to minimum width");
            }
            ResizeEdge::Top => {
                let original_bottom = original.origin.y + original.size.height;
                let actual_bottom = actual.origin.y + actual.size.height;
                assert!((actual_bottom - original_bottom).abs() < 0.01);
                assert!(actual.size.height > 1.0, "Core should clamp to minimum height");
            }
            ResizeEdge::TopLeft => {
                let original_right = original.origin.x + original.size.width;
                let actual_right = actual.origin.x + actual.size.width;
                let original_bottom = original.origin.y + original.size.height;
                let actual_bottom = actual.origin.y + actual.size.height;
                assert!((actual_right - original_right).abs() < 0.01);
                assert!((actual_bottom - original_bottom).abs() < 0.01);
                assert!(actual.size.width > 1.0, "Core should clamp to minimum width");
                assert!(actual.size.height > 1.0, "Core should clamp to minimum height");
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
                ResizeEdge::Left => {
                    Point::new(start.x + original_bounds.size.width - 1.0, start.y)
                }
                ResizeEdge::Top => {
                    Point::new(start.x, start.y + original_bounds.size.height - 1.0)
                }
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
