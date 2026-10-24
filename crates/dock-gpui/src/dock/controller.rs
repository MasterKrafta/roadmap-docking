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
#[path = "controller_tests.rs"]
mod tests;
