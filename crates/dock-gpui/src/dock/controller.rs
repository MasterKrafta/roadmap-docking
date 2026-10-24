use std::collections::HashMap;

use dock_core::{
    create_initial_layout, Axis, CommandError, CommandOutcome, ComputeRectsError, DockCommand,
    DockMetrics, DockSide, DropTarget, DropZone, Layout, Node, NodeId, PanelId, Point, Profile,
    Rect, UndoRedoHistory,
};
use gpui::ScrollHandle;
use ui::Theme;

use crate::{dock_metrics_from_theme, FrameGeometry};
use crate::requests::ResizeHandle;

use super::resize_session::{ResizeSession, ResizeStartError, ResizeUpdateError};

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
        let result = self
            .history
            .apply(&mut self.layout, command, &self.metrics);
        self.track_command_result(result, "apply")
    }

    /// Restores the preceding recorded layout revision, if one exists.
    pub(crate) fn undo(&mut self) -> Result<bool, CommandError> {
        let result = self.history.undo(&mut self.layout, &self.metrics);
        self.track_command_result(result, "undo")
    }

    /// Reapplies the next recorded layout revision, if one exists.
    pub(crate) fn redo(&mut self) -> Result<bool, CommandError> {
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
            Session::Resize(session) => (
                session.baseline().clone(),
                session.command_for_pointer(pointer)?,
            ),
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
        let command = session.command_for_pointer(pointer);
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

#[cfg(test)]
mod tests {
    use super::DockController;
    use crate::dock::resize_session::StartGeometry;
    use crate::requests::ResizeHandle;
    use dock_core::{Axis, Node, Point, Rect, Size};
    use ui::Theme;

    #[test]
    fn initial_render_layout_is_valid() {
        let controller =
            DockController::new_initial(&Theme::default()).expect("initial layout should be valid");
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
        assert_eq!(controller.layout().panel_instances().count(), 4);
    }

    #[test]
    fn begin_resize_captures_start_geometry_and_rejects_a_second_session() {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 800.0));
        let frame = controller.frame(viewport).unwrap();
        let splitter = *frame
            .rects()
            .splitters
            .iter()
            .find(|splitter| splitter.axis == Axis::Y)
            .expect("the initial layout should contain a vertical group splitter");
        let group_count = match controller.layout().node(splitter.owner) {
            Some(Node::Column(column)) => column.groups.len(),
            other => panic!("vertical splitter should belong to a column, got {other:?}"),
        };
        let column_rect = frame
            .rects()
            .columns
            .iter()
            .find(|column| column.column == splitter.owner)
            .expect("splitter column should have geometry");
        let expected_available_height = (column_rect.rect.size.height
            - controller.metrics().splitter_thickness * (group_count - 1) as f32)
            .max(0.0);
        let handle = ResizeHandle::Splitter {
            owner: splitter.owner,
            boundary_index: splitter.boundary_index,
            axis: Axis::Y,
        };
        let pointer = Point::new(180.0, 240.0);

        controller.begin_resize(handle, pointer, &frame).unwrap();

        let session = controller.resize_session().unwrap();
        assert_eq!(session.handle(), handle);
        assert_eq!(session.pointer_start(), pointer);
        assert_eq!(
            session.start_geometry(),
            StartGeometry::VerticalGroups {
                available_height: expected_available_height,
            }
        );
        assert_eq!(session.baseline().panel_instances().count(), 4);
        assert_eq!(
            controller.begin_resize(handle, pointer, &frame),
            Err(super::super::resize_session::ResizeStartError::SessionAlreadyActive)
        );
    }


    #[test]
    fn committing_resize_records_one_undo_step_for_the_whole_gesture() {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();
        let splitter = *frame
            .rects()
            .splitters
            .iter()
            .find(|splitter| splitter.axis == Axis::Y)
            .expect("initial layout should contain a vertical group splitter");
        let owner = splitter.owner;
        let original_ratios = match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            other => panic!("vertical splitter should belong to a column, got {other:?}"),
        };
        let start = Point::new(180.0, 240.0);
        controller
            .begin_resize(
                ResizeHandle::Splitter {
                    owner,
                    boundary_index: splitter.boundary_index,
                    axis: Axis::Y,
                },
                start,
                &frame,
            )
            .unwrap();

        controller
            .update_resize(Point::new(start.x, start.y + 16.0))
            .unwrap();
        let preview_ratios = match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => panic!("resize owner should remain a column"),
        };
        controller
            .commit_resize(Point::new(start.x, start.y + 16.0))
            .unwrap();

        assert_eq!(
            match controller.layout().node(owner) {
                Some(Node::Column(column)) => column.group_ratios.clone(),
                _ => panic!("resize owner should remain a column"),
            },
            preview_ratios
        );
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));
        assert!(controller.resize_session().is_none());
        assert!(controller.undo().unwrap());
        assert_eq!(
            match controller.layout().node(owner) {
                Some(Node::Column(column)) => column.group_ratios.clone(),
                _ => panic!("resize owner should remain a column after undo"),
            },
            original_ratios
        );
        assert!(!controller.undo().unwrap(), "one resize should create one undo step");
    }

    #[test]
    fn cancelling_resize_restores_baseline_and_clears_the_session() {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();
        let splitter = *frame
            .rects()
            .splitters
            .iter()
            .find(|splitter| splitter.axis == Axis::Y)
            .expect("initial layout should contain a vertical group splitter");
        let owner = splitter.owner;
        let original_ratios = match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            other => panic!("vertical splitter should belong to a column, got {other:?}"),
        };
        let baseline_version = controller.layout().version();
        let start = Point::new(180.0, 240.0);
        controller
            .begin_resize(
                ResizeHandle::Splitter {
                    owner,
                    boundary_index: splitter.boundary_index,
                    axis: Axis::Y,
                },
                start,
                &frame,
            )
            .unwrap();
        controller
            .update_resize(Point::new(start.x, start.y + 16.0))
            .unwrap();
        assert!(controller.cancel_resize());

        assert_eq!(controller.layout().version(), baseline_version);
        assert_eq!(
            match controller.layout().node(owner) {
                Some(Node::Column(column)) => column.group_ratios.clone(),
                _ => panic!("resize owner should remain a column after cancel"),
            },
            original_ratios
        );
        assert!(controller.resize_session().is_none());
        assert!(!controller.cancel_resize());
        assert!(matches!(
            controller.update_resize(start),
            Err(super::super::resize_session::ResizeUpdateError::NoActiveSession)
        ));
        assert!(!controller.undo().unwrap());
    }

    #[test]
    fn resize_preview_is_recomputed_from_baseline_and_zero_delta_restores_it() {
        let mut controller = DockController::new_initial(&Theme::default()).unwrap();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();
        let splitter = *frame
            .rects()
            .splitters
            .iter()
            .find(|splitter| splitter.axis == Axis::Y)
            .expect("initial layout should contain a vertical group splitter");
        let owner = splitter.owner;
        let original = match controller.layout().node(owner) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            other => panic!("vertical splitter should belong to a column, got {other:?}"),
        };
        let initial_version = controller.layout().version();
        let start = Point::new(180.0, 240.0);
        controller
            .begin_resize(
                ResizeHandle::Splitter {
                    owner,
                    boundary_index: splitter.boundary_index,
                    axis: Axis::Y,
                },
                start,
                &frame,
            )
            .unwrap();

        let preview = controller
            .update_resize(Point::new(start.x, start.y + 12.0))
            .unwrap();
        assert_eq!(controller.layout().version(), preview.new_version);
        assert_eq!(controller.layout().validate(controller.metrics()), Ok(()));

        controller.update_resize(start).unwrap();
        assert_eq!(controller.layout().version(), initial_version);
        assert_eq!(
            match controller.layout().node(owner) {
                Some(Node::Column(column)) => column.group_ratios.clone(),
                _ => panic!("resize owner should remain a column"),
            },
            original
        );
    }
}
