use crate::command::DropTarget;
use crate::geometry::{DropPayload, Point};

/// Drag recognition starts after four logical pixels of pointer movement.
pub const DRAG_START_THRESHOLD: f32 = 4.0;

/// Why an in-progress drag was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DragCancelReason {
    /// The user pressed Escape.
    Escape,
    /// The application window lost focus while a pointer was held.
    WindowFocusLost,
    /// The pointer was released after dragging without a valid drop target.
    ReleasedOutsideTarget,
    /// The pointer was released before crossing the drag threshold.
    ReleasedBeforeThreshold,
    /// The caller explicitly cancelled the active interaction.
    Explicit,
}

/// Pure state of the pointer-driven drag lifecycle.
///
/// Coordinates are logical pixels. The session tracks only an intent and a preview
/// target; it never mutates Layout or applies a DockCommand.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DragSessionState {
    /// No pointer interaction is active.
    Idle,
    /// Pointer is down, but movement has not crossed the drag threshold.
    Pressed { payload: DropPayload, origin: Point },
    /// The drag threshold was crossed. The current target is preview-only.
    Dragging {
        payload: DropPayload,
        origin: Point,
        position: Point,
        target: Option<DropTarget>,
    },
    /// A valid release produced one drop request.
    Dropped {
        payload: DropPayload,
        target: DropTarget,
    },
    /// The active interaction ended without a drop request.
    Cancelled { reason: DragCancelReason },
}

/// One accepted release. The UI adapter translates this request into exactly one
/// DockCommand; a DragSession never changes the layout itself.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DragDropRequest {
    pub payload: DropPayload,
    pub target: DropTarget,
}

/// Framework-independent drag recognizer for a single pointer interaction.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DragSession {
    state: DragSessionState,
}

impl Default for DragSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DragSession {
    pub const fn new() -> Self {
        Self {
            state: DragSessionState::Idle,
        }
    }

    pub fn state(&self) -> &DragSessionState {
        &self.state
    }

    pub fn is_dragging(&self) -> bool {
        matches!(&self.state, DragSessionState::Dragging { .. })
    }

    /// Begin tracking a pointer press on a draggable payload.
    ///
    /// A new interaction can start from Idle or a terminal state. A press during
    /// Pressed/Dragging is ignored so one pointer sequence cannot create two drags.
    pub fn press(&mut self, payload: DropPayload, origin: Point) -> bool {
        if !is_finite_point(origin)
            || matches!(
                &self.state,
                DragSessionState::Pressed { .. } | DragSessionState::Dragging { .. }
            )
        {
            return false;
        }

        self.state = DragSessionState::Pressed { payload, origin };
        true
    }

    /// Feed a pointer-move event in logical coordinates.
    ///
    /// Before the threshold is crossed, target is ignored and no preview is shown.
    /// While Dragging, only the preview state changes; the dock tree is untouched.
    /// Returns true when the session is actively dragging after this event.
    pub fn pointer_moved(&mut self, position: Point, target: Option<DropTarget>) -> bool {
        if !is_finite_point(position) {
            return self.is_dragging();
        }

        match self.state.clone() {
            DragSessionState::Pressed { payload, origin } => {
                let dx = position.x - origin.x;
                let dy = position.y - origin.y;
                let threshold_squared = DRAG_START_THRESHOLD * DRAG_START_THRESHOLD;

                if dx * dx + dy * dy < threshold_squared {
                    return false;
                }

                self.state = DragSessionState::Dragging {
                    payload,
                    origin,
                    position,
                    target,
                };
                true
            }
            DragSessionState::Dragging {
                payload, origin, ..
            } => {
                self.state = DragSessionState::Dragging {
                    payload,
                    origin,
                    position,
                    target,
                };
                true
            }
            DragSessionState::Idle
            | DragSessionState::Dropped { .. }
            | DragSessionState::Cancelled { .. } => false,
        }
    }

    /// Current semantic drop target for an overlay preview, if any.
    pub fn preview_target(&self) -> Option<&DropTarget> {
        match &self.state {
            DragSessionState::Dragging { target, .. } => target.as_ref(),
            DragSessionState::Idle
            | DragSessionState::Pressed { .. }
            | DragSessionState::Dropped { .. }
            | DragSessionState::Cancelled { .. } => None,
        }
    }

    /// End the pointer sequence. A valid drag yields at most one drop request.
    ///
    /// Releasing while merely Pressed is a click, not a drag. Releasing a drag without
    /// a valid target cancels it. Further releases in a terminal state yield nothing.
    pub fn release(&mut self) -> Option<DragDropRequest> {
        match self.state.clone() {
            DragSessionState::Dragging {
                payload,
                target: Some(target),
                ..
            } => {
                self.state = DragSessionState::Dropped {
                    payload,
                    target: target.clone(),
                };
                Some(DragDropRequest { payload, target })
            }
            DragSessionState::Dragging { target: None, .. } => {
                self.state = DragSessionState::Cancelled {
                    reason: DragCancelReason::ReleasedOutsideTarget,
                };
                None
            }
            DragSessionState::Pressed { .. } => {
                self.state = DragSessionState::Cancelled {
                    reason: DragCancelReason::ReleasedBeforeThreshold,
                };
                None
            }
            DragSessionState::Idle
            | DragSessionState::Dropped { .. }
            | DragSessionState::Cancelled { .. } => None,
        }
    }

    /// Cancel a Pressed or Dragging sequence without producing a drop request.
    pub fn cancel(&mut self, reason: DragCancelReason) -> bool {
        if !matches!(
            &self.state,
            DragSessionState::Pressed { .. } | DragSessionState::Dragging { .. }
        ) {
            return false;
        }

        self.state = DragSessionState::Cancelled { reason };
        true
    }
}

fn is_finite_point(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::{DRAG_START_THRESHOLD, DragCancelReason, DragSession, DragSessionState};
    use crate::{
        CanvasState, DockCommand, DockMetrics, DropPayload, DropTarget, DropZone, Layout, PanelId,
        PanelKey, Point, Profile, Rect, Size,
    };
    use slotmap::Key;

    fn panel_payload() -> DropPayload {
        DropPayload::Panel(PanelId::null())
    }

    fn target() -> DropTarget {
        DropTarget::Group {
            group: crate::NodeId::null(),
            zone: DropZone::Center,
        }
    }

    fn metrics() -> DockMetrics {
        DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        }
    }

    #[test]
    fn starts_idle_and_uses_four_logical_pixels_as_the_threshold() {
        let mut drag = DragSession::new();
        assert_eq!(drag.state(), &DragSessionState::Idle);
        assert_eq!(DRAG_START_THRESHOLD, 4.0);

        assert!(drag.press(panel_payload(), Point::new(10.0, 20.0)));
        assert!(matches!(drag.state(), DragSessionState::Pressed { .. }));

        assert!(!drag.pointer_moved(Point::new(13.99, 20.0), Some(target())));
        assert!(matches!(drag.state(), DragSessionState::Pressed { .. }));
        assert!(drag.preview_target().is_none());

        assert!(drag.pointer_moved(Point::new(14.0, 20.0), Some(target())));
        assert!(drag.is_dragging());
        assert_eq!(drag.preview_target(), Some(&target()));
    }

    #[test]
    fn release_below_threshold_is_a_click_and_emits_no_drop() {
        let mut drag = DragSession::new();
        assert!(drag.press(panel_payload(), Point::new(3.0, 4.0)));
        assert!(!drag.pointer_moved(Point::new(4.0, 4.0), Some(target())));

        assert!(drag.release().is_none());
        assert_eq!(
            drag.state(),
            &DragSessionState::Cancelled {
                reason: DragCancelReason::ReleasedBeforeThreshold
            }
        );
        assert!(drag.release().is_none());
    }

    #[test]
    fn valid_release_produces_one_drop_request_and_enters_dropped_state() {
        let payload = panel_payload();
        let destination = target();
        let mut drag = DragSession::new();
        assert!(drag.press(payload, Point::new(0.0, 0.0)));
        assert!(drag.pointer_moved(Point::new(4.0, 0.0), Some(destination.clone())));

        let drop = drag
            .release()
            .expect("valid release should produce a drop request");
        assert_eq!(drop.payload, payload);
        assert_eq!(drop.target, destination);
        assert_eq!(
            drag.state(),
            &DragSessionState::Dropped {
                payload,
                target: destination
            }
        );
        assert!(
            drag.release().is_none(),
            "a second release must emit nothing"
        );
        assert!(!drag.pointer_moved(Point::new(20.0, 0.0), Some(target())));
    }

    #[test]
    fn releasing_a_drag_without_a_valid_target_cancels_it() {
        let mut drag = DragSession::new();
        assert!(drag.press(panel_payload(), Point::default()));
        assert!(drag.pointer_moved(Point::new(4.0, 0.0), None));

        assert!(drag.release().is_none());
        assert_eq!(
            drag.state(),
            &DragSessionState::Cancelled {
                reason: DragCancelReason::ReleasedOutsideTarget
            }
        );
    }

    #[test]
    fn escape_and_window_focus_loss_cancel_without_a_drop_request_or_layout_change() {
        let metrics = metrics();
        let mut layout = Layout::new(Profile::Photoshop, CanvasState::default());
        // Add a real opened panel so the unchanged version/placement checks are meaningful.
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: PanelKey::new("layers").expect("valid panel key"),
                    constraints: crate::PanelConstraints::new(
                        Size::new(100.0, 80.0),
                        Size::new(160.0, 120.0),
                        Size::new(640.0, 480.0),
                        true,
                    ),
                    target: None,
                },
                &metrics,
            )
            .expect("test panel opens");

        let panel = layout.panels().iter().next().map(|(id, _)| id).unwrap();
        let payload = DropPayload::Panel(panel);
        let version_before = layout.version();
        let panel_count_before = layout.panels().len();

        for reason in [DragCancelReason::Escape, DragCancelReason::WindowFocusLost] {
            let mut drag = DragSession::new();
            assert!(drag.press(payload, Point::new(20.0, 30.0)));
            assert!(drag.pointer_moved(Point::new(24.0, 30.0), Some(target())));
            assert!(drag.cancel(reason));
            assert!(drag.release().is_none());

            assert_eq!(drag.state(), &DragSessionState::Cancelled { reason });
            assert_eq!(layout.version(), version_before);
            assert_eq!(layout.panels().len(), panel_count_before);
            assert!(layout.validate(&metrics).is_ok());
        }
    }

    #[test]
    fn cancel_while_pressed_or_dragging_is_terminal_until_a_new_press() {
        let mut drag = DragSession::new();
        assert!(drag.press(panel_payload(), Point::default()));
        assert!(drag.cancel(DragCancelReason::Escape));
        assert!(!drag.cancel(DragCancelReason::WindowFocusLost));
        assert!(!drag.pointer_moved(Point::new(10.0, 0.0), Some(target())));

        // A new pointer press starts a fresh interaction.
        assert!(drag.press(panel_payload(), Point::new(5.0, 5.0)));
        assert!(matches!(drag.state(), DragSessionState::Pressed { .. }));
    }

    #[test]
    fn invalid_pointer_coordinates_are_ignored() {
        let mut drag = DragSession::new();
        assert!(!drag.press(panel_payload(), Point::new(f32::NAN, 0.0)));
        assert_eq!(drag.state(), &DragSessionState::Idle);

        assert!(drag.press(panel_payload(), Point::default()));
        assert!(!drag.pointer_moved(Point::new(f32::INFINITY, 0.0), Some(target())));
        assert!(matches!(drag.state(), DragSessionState::Pressed { .. }));
    }

    #[test]
    fn drag_requests_keep_semantic_targets_without_mutating_layout() {
        let metrics = metrics();
        let mut layout = Layout::new(Profile::Photoshop, CanvasState::default());
        let panel_key = PanelKey::new("layers").expect("valid panel key");
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: panel_key,
                    constraints: crate::PanelConstraints::new(
                        Size::new(100.0, 80.0),
                        Size::new(160.0, 120.0),
                        Size::new(640.0, 480.0),
                        true,
                    ),
                    target: None,
                },
                &metrics,
            )
            .expect("test panel opens");
        let panel = layout.panels().iter().next().map(|(id, _)| id).unwrap();
        let version_before = layout.version();

        let destination = DropTarget::FreeArea {
            bounds: Rect::new(Point::new(20.0, 30.0), Size::new(240.0, 160.0)),
        };
        let mut drag = DragSession::new();
        assert!(drag.press(DropPayload::Panel(panel), Point::new(10.0, 10.0)));
        assert!(drag.pointer_moved(Point::new(14.0, 10.0), Some(destination.clone())));
        assert_eq!(
            layout.version(),
            version_before,
            "preview must not mutate the dock tree"
        );

        let request = drag.release().expect("valid drop emits one request");
        assert_eq!(request.target, destination);
        assert_eq!(
            layout.version(),
            version_before,
            "the core session is intent-only"
        );
    }
}
