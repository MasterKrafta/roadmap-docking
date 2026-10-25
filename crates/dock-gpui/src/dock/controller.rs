use std::collections::HashMap;

use dock_core::{
    create_initial_layout, Axis, CommandError, CommandOutcome, ComputeRectsError, DockCommand,
    DockMetrics, DockSide, DropTarget, DropZone, Layout, Node, NodeId, PanelId, Point, Profile,
    Rect, ResizeEdge, UndoRedoHistory,
};
use gpui::ScrollHandle;
use ui::Theme;

use crate::{dock_metrics_from_theme, FrameGeometry};
use crate::requests::ResizeHandle;

use super::resize_session::{ResizeSession, ResizeStartError, ResizeUpdateError, StartGeometry};

#[derive(Debug)]
enum Session {
    Idle,
    Resize(ResizeSession),
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
        let result = self
            .history
            .apply(&mut self.layout, command, &self.metrics);
        self.track_command_result(result, "apply")
    }

    /// Restores the preceding recorded layout revision, if one exists.
    pub(crate) fn undo(&mut self) -> Result<bool, CommandError> {
        self.cancel_resize();
        let result = self.history.undo(&mut self.layout, &self.metrics);
        self.track_command_result(result, "undo")
    }

    /// Reapplies the next recorded layout revision, if one exists.
    pub(crate) fn redo(&mut self) -> Result<bool, CommandError> {
        self.cancel_resize();
        let result = self.history.redo(&mut self.layout, &self.metrics);
        self.track_command_result(result, "redo")
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
            Session::Idle => None,
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
            Session::Idle => return Err(ResizeUpdateError::NoActiveSession),
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
                Err(ResizeUpdateError::Command(error))
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
        let session = match std::mem::replace(&mut self.session, Session::Idle) {
            Session::Resize(session) => session,
            Session::Idle => return Err(ResizeUpdateError::NoActiveSession),
        };
        let command = session
            .command_for_pointer(pointer)
            .map(|command| preserve_floating_resize_anchor(&session, command, &self.metrics));
        self.layout = session.into_baseline();
        self.last_error = None;

        let command = command?;
        let result = self.history.apply(&mut self.layout, command, &self.metrics);
        self.track_command_result(result, "resize_commit")
            .map_err(ResizeUpdateError::Command)
    }

    /// Cancels an active resize and restores the exact pointer-down snapshot.
    ///
    /// Returns false when there is no resize session to cancel.
    pub(crate) fn cancel_resize(&mut self) -> bool {
        match std::mem::replace(&mut self.session, Session::Idle) {
            Session::Resize(session) => {
                self.layout = session.into_baseline();
                self.last_error = None;
                true
            }
            Session::Idle => false,
        }
    }

    /// Computes the normalized frame geometry for the current layout and viewport.
    pub(crate) fn frame(&self, viewport: Rect) -> Result<FrameGeometry, ComputeRectsError> {
        FrameGeometry::compute(&self.layout, viewport, &self.metrics)
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

/// Builds the starting arrangement inside the controller so no other module
/// mutates a live Layout. The setup commands are deliberately outside history.
fn build_initial_layout(theme: &Theme) -> Result<(Layout, DockMetrics), String> {
    let metrics = dock_metrics_from_theme(theme);
    let descriptors = crate::panels::descriptors(theme);
    let mut layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .map_err(|error| format!("{error:?}"))?;

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
            _ => unreachable!("test covers the left and top edges"),
        }
    }

    #[test]
    fn floating_left_and_top_resize_keep_opposite_edge_anchored_at_minimum() {
        for edge in [ResizeEdge::Left, ResizeEdge::Top] {
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
                _ => unreachable!("test covers the left and top edges"),
            };
            let end = match edge {
                ResizeEdge::Left => {
                    Point::new(start.x + original_bounds.size.width - 1.0, start.y)
                }
                ResizeEdge::Top => {
                    Point::new(start.x, start.y + original_bounds.size.height - 1.0)
                }
                _ => unreachable!("test covers the left and top edges"),
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
