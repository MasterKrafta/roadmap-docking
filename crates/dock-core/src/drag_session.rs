use crate::{
    DockMetrics, DragCancelReason, DragDropRequest, DragPayload, DragPreview,
    DragSessionError, DragState, DropTarget, Point,
};

/// A deterministic, UI-independent drag gesture state machine.
///
/// The threshold is copied from `DockMetrics::drag_threshold` at creation,
/// so it is configurable and stable for the lifetime of this gesture session.
/// Pointer positions and the threshold are measured in logical pixels.
///
/// A session never owns or mutates `Layout`. It exposes preview data while
/// dragging and emits at most one semantic `DragDropRequest` on valid release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragSession {
    state: DragState,
    threshold: f32,
}

impl DragSession {
    /// Creates an idle session using the configured logical-pixel drag threshold.
    pub fn new(metrics: &DockMetrics) -> Result<Self, DragSessionError> {
        let threshold = metrics.drag_threshold;
        if !threshold.is_finite() || threshold <= 0.0 {
            return Err(DragSessionError::InvalidThreshold);
        }

        Ok(Self {
            state: DragState::Idle,
            threshold,
        })
    }

    /// Returns the complete current state by value.
    pub fn state(&self) -> DragState {
        self.state
    }

    /// Returns the threshold snapshot used by this session.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Starts tracking a pressed pointer. This does not start dragging.
    pub fn pointer_down(
        &mut self,
        payload: DragPayload,
        position: Point,
    ) -> Result<(), DragSessionError> {
        if !matches!(self.state, DragState::Idle) {
            return Err(DragSessionError::SessionNotIdle);
        }
        Self::validate_position(position)?;

        self.state = DragState::Pressed {
            payload,
            origin: position,
            position,
        };
        Ok(())
    }

    /// Updates the pointer position and begins dragging once the Euclidean
    /// distance from the original press reaches the configured threshold.
    ///
    /// Moving after a target was resolved clears that target. The adapter must
    /// resolve the target again for the new point before release.
    pub fn pointer_move(&mut self, position: Point) -> Result<(), DragSessionError> {
        Self::validate_position(position)?;

        self.state = match self.state {
            DragState::Pressed {
                payload, origin, ..
            } => {
                if Self::threshold_reached(origin, position, self.threshold) {
                    DragState::Dragging {
                        payload,
                        origin,
                        position,
                        target: None,
                    }
                } else {
                    DragState::Pressed {
                        payload,
                        origin,
                        position,
                    }
                }
            }
            DragState::Dragging {
                payload, origin, ..
            } => DragState::Dragging {
                payload,
                origin,
                position,
                target: None,
            },
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                return Err(DragSessionError::NoActiveGesture);
            }
        };
        Ok(())
    }

    /// Stores the currently resolved target for the drag preview.
    ///
    /// Pass `None` when the pointer is outside actionable targets. The target
    /// is only a transient preview; command application must revalidate IDs.
    pub fn set_drop_target(
        &mut self,
        target: Option<DropTarget>,
    ) -> Result<(), DragSessionError> {
        match self.state {
            DragState::Dragging {
                payload,
                origin,
                position,
                ..
            } => {
                self.state = DragState::Dragging {
                    payload,
                    origin,
                    position,
                    target,
                };
                Ok(())
            }
            _ => Err(DragSessionError::NotDragging),
        }
    }

    /// Returns preview point and its latest resolved target only while dragging.
    pub fn preview(&self) -> Option<DragPreview> {
        match self.state {
            DragState::Dragging {
                payload,
                position,
                target,
                ..
            } => Some(DragPreview {
                payload,
                point: position,
                target,
            }),
            _ => None,
        }
    }

    /// Releases the pointer using the target freshly resolved for the release
    /// point. A valid drag produces exactly one request; a click or a release
    /// without a target is cancelled and produces no request.
    ///
    /// The caller should pass `None` when the pointer was released outside any
    /// valid target. The returned request does not mutate Layout; an adapter
    /// translates it into one DockCommand and applies that command once.
    pub fn pointer_up(
        &mut self,
        position: Point,
        resolved_target: Option<DropTarget>,
    ) -> Result<Option<DragDropRequest>, DragSessionError> {
        Self::validate_position(position)?;

        let current_state = self.state;
        let request = match current_state {
            DragState::Pressed { payload, origin, .. } => {
                if !Self::threshold_reached(origin, position, self.threshold) {
                    self.state = DragState::Cancelled {
                        payload,
                        origin,
                        position,
                        reason: DragCancelReason::BelowThreshold,
                    };
                    return Ok(None);
                }

                Self::finish_release(payload, origin, position, resolved_target, &mut self.state)
            }
            DragState::Dragging {
                payload, origin, ..
            } => Self::finish_release(
                payload,
                origin,
                position,
                resolved_target,
                &mut self.state,
            ),
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                return Err(DragSessionError::NoActiveGesture);
            }
        };

        Ok(request)
    }

    /// Cancels a pressed or dragging gesture with an explicit generic reason.
    pub fn cancel(&mut self) -> Result<(), DragSessionError> {
        self.cancel_with_reason(DragCancelReason::Explicit)
    }

    /// Cancels the current gesture when the adapter receives Escape.
    pub fn escape(&mut self) -> Result<(), DragSessionError> {
        self.cancel_with_reason(DragCancelReason::Escape)
    }

    /// Cancels the current gesture when the input surface loses focus.
    pub fn focus_lost(&mut self) -> Result<(), DragSessionError> {
        self.cancel_with_reason(DragCancelReason::FocusLost)
    }

    fn cancel_with_reason(
        &mut self,
        reason: DragCancelReason,
    ) -> Result<(), DragSessionError> {
        self.state = match self.state {
            DragState::Pressed {
                payload,
                origin,
                position,
            }
            | DragState::Dragging {
                payload,
                origin,
                position,
                ..
            } => DragState::Cancelled {
                payload,
                origin,
                position,
                reason,
            },
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                return Err(DragSessionError::NoActiveGesture);
            }
        };
        Ok(())
    }

    /// Returns a completed session to idle so it can track another gesture.
    /// An in-progress gesture must first be dropped or cancelled.
    pub fn reset(&mut self) -> Result<(), DragSessionError> {
        match self.state {
            DragState::Pressed { .. } | DragState::Dragging { .. } => {
                Err(DragSessionError::CannotResetActiveGesture)
            }
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                self.state = DragState::Idle;
                Ok(())
            }
        }
    }

    fn finish_release(
        payload: DragPayload,
        origin: Point,
        position: Point,
        target: Option<DropTarget>,
        state: &mut DragState,
    ) -> Option<DragDropRequest> {
        let Some(target) = target else {
            *state = DragState::Cancelled {
                payload,
                origin,
                position,
                reason: DragCancelReason::NoValidTarget,
            };
            return None;
        };

        *state = DragState::Dropped {
            payload,
            origin,
            position,
            target,
        };
        Some(DragDropRequest {
            payload,
            origin,
            point: position,
            target,
        })
    }

    fn validate_position(position: Point) -> Result<(), DragSessionError> {
        if position.is_finite() {
            Ok(())
        } else {
            Err(DragSessionError::InvalidPointerPosition)
        }
    }

    fn threshold_reached(origin: Point, position: Point, threshold: f32) -> bool {
        let dx = position.x - origin.x;
        let dy = position.y - origin.y;
        dx.hypot(dy) >= threshold
    }
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use super::DragSession;
    use crate::{
        create_initial_layout, DockAreaId, DockMetrics, DragCancelReason, DragDropRequest,
        DragPayload, DragPreview, DragSessionError, DragState, DropTarget, FloatingGroupId,
        Layout, Node, NodeId, PanelConstraints, PanelId, PanelKey, Point, Profile, Size,
    };

    fn panel_payload() -> DragPayload {
        let mut panels = SlotMap::<PanelId, ()>::with_key();
        DragPayload::Panel {
            panel: panels.insert(()),
        }
    }

    fn session() -> DragSession {
        DragSession::new(&DockMetrics::default()).expect("default drag metrics are valid")
    }

    fn test_layout() -> Layout {
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        create_initial_layout(Profile::Photoshop, &descriptors, DockMetrics::default()).unwrap()
    }

    fn layout_snapshot(
        layout: &Layout,
    ) -> (
        u64,
        String,
        Vec<(NodeId, String)>,
        Vec<(PanelId, String)>,
        Vec<(DockAreaId, String)>,
        Vec<(FloatingGroupId, String)>,
    ) {
        (
            layout.version(),
            format!(
                "{:?}|{:?}|{:?}|{:?}",
                layout.profile(),
                layout.chrome(),
                layout.canvas(),
                layout.flyout()
            ),
            layout
                .nodes()
                .map(|(id, node)| (id, format!("{node:?}")))
                .collect(),
            layout
                .panel_instances()
                .map(|(id, panel)| (id, format!("{panel:?}")))
                .collect(),
            layout
                .dock_areas()
                .map(|(id, area)| (id, format!("{area:?}")))
                .collect(),
            layout
                .floating_groups()
                .map(|(id, group)| (id, format!("{group:?}")))
                .collect(),
        )
    }

    fn target_for_layout(layout: &Layout) -> DropTarget {
        let group = layout
            .nodes()
            .find_map(|(id, node)| matches!(node, Node::Tabs(_)).then_some(id))
            .expect("baseline layout contains a tab group");
        DropTarget::Tabs { group, index: 0 }
    }

    #[test]
    fn pointer_down_is_pressed_and_click_below_threshold_is_cancelled() {
        let payload = panel_payload();
        let mut session = session();

        session
            .pointer_down(payload, Point::new(10.0, 20.0))
            .unwrap();
        assert_eq!(
            session.state(),
            DragState::Pressed {
                payload,
                origin: Point::new(10.0, 20.0),
                position: Point::new(10.0, 20.0),
            }
        );

        session
            .pointer_move(Point::new(13.9, 20.0))
            .unwrap();
        assert!(matches!(session.state(), DragState::Pressed { .. }));

        assert_eq!(session.pointer_up(Point::new(13.5, 20.0), Some(DropTarget::FreeArea)).unwrap(), None);
        assert_eq!(
            session.state(),
            DragState::Cancelled {
                payload,
                origin: Point::new(10.0, 20.0),
                position: Point::new(13.5, 20.0),
                reason: DragCancelReason::BelowThreshold,
            }
        );
    }

    #[test]
    fn preview_exposes_point_and_optional_target_without_mutating_layout() {
        let layout = test_layout();
        let before = layout_snapshot(&layout);
        let payload = panel_payload();
        let mut session = session();

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 1.0)).unwrap();
        assert_eq!(
            session.preview(),
            Some(DragPreview {
                payload,
                point: Point::new(4.0, 1.0),
                target: None,
            })
        );

        let target = target_for_layout(&layout);
        session.set_drop_target(Some(target)).unwrap();
        assert_eq!(
            session.preview(),
            Some(DragPreview {
                payload,
                point: Point::new(4.0, 1.0),
                target: Some(target),
            })
        );
        assert_eq!(layout_snapshot(&layout), before);
    }

    #[test]
    fn valid_release_emits_exactly_one_request() {
        let layout = test_layout();
        let before = layout_snapshot(&layout);
        let payload = panel_payload();
        let target = target_for_layout(&layout);
        let mut session = session();
        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 0.0)).unwrap();
        session.set_drop_target(Some(target)).unwrap();

        let request = session
            .pointer_up(Point::new(8.0, 2.0), Some(target))
            .unwrap()
            .expect("valid release emits one request");
        assert_eq!(
            request,
            DragDropRequest {
                payload,
                origin: Point::ZERO,
                point: Point::new(8.0, 2.0),
                target,
            }
        );
        assert_eq!(
            session.state(),
            DragState::Dropped {
                payload,
                origin: Point::ZERO,
                position: Point::new(8.0, 2.0),
                target,
            }
        );
        assert_eq!(
            session.pointer_up(Point::new(8.0, 2.0), Some(target)),
            Err(DragSessionError::NoActiveGesture),
            "a terminal gesture must not emit a second request"
        );
        assert_eq!(layout_snapshot(&layout), before);
    }

    #[test]
    fn cancel_and_release_without_target_emit_no_request_and_preserve_layout() {
        let layout = test_layout();
        let before = layout_snapshot(&layout);
        let payload = panel_payload();
        let mut session = session();

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 0.0)).unwrap();
        assert_eq!(session.pointer_up(Point::new(5.0, 0.0), None).unwrap(), None);
        assert_eq!(
            session.state(),
            DragState::Cancelled {
                payload,
                origin: Point::ZERO,
                position: Point::new(5.0, 0.0),
                reason: DragCancelReason::NoValidTarget,
            }
        );
        assert_eq!(layout_snapshot(&layout), before);

        session.reset().unwrap();
        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 0.0)).unwrap();
        session.escape().unwrap();
        assert_eq!(
            session.state(),
            DragState::Cancelled {
                payload,
                origin: Point::ZERO,
                position: Point::new(4.0, 0.0),
                reason: DragCancelReason::Escape,
            }
        );
        assert_eq!(layout_snapshot(&layout), before);
    }

    #[test]
    fn focus_loss_cancels_without_a_drop_request() {
        let payload = panel_payload();
        let mut session = session();
        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 0.0)).unwrap();

        session.focus_lost().unwrap();
        assert!(matches!(
            session.state(),
            DragState::Cancelled {
                reason: DragCancelReason::FocusLost,
                ..
            }
        ));
        assert_eq!(
            session.pointer_up(Point::new(4.0, 0.0), None),
            Err(DragSessionError::NoActiveGesture)
        );
    }

    #[test]
    fn release_position_can_cross_threshold_without_an_intermediate_move_event() {
        let payload = panel_payload();
        let mut session = session();

        session.pointer_down(payload, Point::ZERO).unwrap();
        let request = session
            .pointer_up(Point::new(0.0, 4.0), Some(DropTarget::FreeArea))
            .unwrap()
            .expect("release point crosses threshold");
        assert_eq!(request.point, Point::new(0.0, 4.0));
        assert_eq!(request.target, DropTarget::FreeArea);
    }

    #[test]
    fn threshold_uses_euclidean_logical_distance_and_is_configurable() {
        let mut metrics = DockMetrics::default();
        metrics.drag_threshold = 5.0;
        let mut session = DragSession::new(&metrics).unwrap();

        assert_eq!(session.threshold(), 5.0);
        let payload = panel_payload();
        session.pointer_down(payload, Point::ZERO).unwrap();

        session.pointer_move(Point::new(3.0, 3.0)).unwrap();
        assert!(matches!(session.state(), DragState::Pressed { .. }));

        session.pointer_move(Point::new(3.0, 4.0)).unwrap();
        assert!(matches!(session.state(), DragState::Dragging { .. }));
        assert_eq!(session.preview().unwrap().target, None);
    }

    #[test]
    fn cancellation_works_before_and_after_drag_and_reset_reuses_session() {
        let payload = panel_payload();
        let mut session = session();

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.cancel().unwrap();
        assert!(matches!(
            session.state(),
            DragState::Cancelled {
                reason: DragCancelReason::Explicit,
                ..
            }
        ));

        session.reset().unwrap();
        assert_eq!(session.state(), DragState::Idle);

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 0.0)).unwrap();
        session.cancel().unwrap();
        assert!(matches!(session.state(), DragState::Cancelled { .. }));

        session.reset().unwrap();
        assert_eq!(session.state(), DragState::Idle);
    }

    #[test]
    fn moving_clears_a_stale_target_and_target_cannot_be_set_before_drag() {
        let payload = panel_payload();
        let mut session = session();
        assert_eq!(
            session.set_drop_target(Some(DropTarget::FreeArea)),
            Err(DragSessionError::NotDragging)
        );

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 0.0)).unwrap();
        session.set_drop_target(Some(DropTarget::FreeArea)).unwrap();
        session.pointer_move(Point::new(4.5, 0.0)).unwrap();
        assert_eq!(session.preview().unwrap().target, None);
    }

    #[test]
    fn invalid_positions_threshold_and_transitions_are_rejected_without_mutation() {
        let mut metrics = DockMetrics::default();
        metrics.drag_threshold = f32::NAN;
        assert_eq!(
            DragSession::new(&metrics),
            Err(DragSessionError::InvalidThreshold)
        );

        let mut session = session();
        assert_eq!(
            session.pointer_down(panel_payload(), Point::new(f32::NAN, 0.0)),
            Err(DragSessionError::InvalidPointerPosition)
        );
        assert_eq!(session.state(), DragState::Idle);
        assert_eq!(
            session.pointer_move(Point::ZERO),
            Err(DragSessionError::NoActiveGesture)
        );

        let payload = panel_payload();
        session.pointer_down(payload, Point::ZERO).unwrap();
        let before = session.state();

        assert_eq!(
            session.pointer_move(Point::new(f32::INFINITY, 0.0)),
            Err(DragSessionError::InvalidPointerPosition)
        );
        assert_eq!(session.state(), before);
        assert_eq!(
            session.reset(),
            Err(DragSessionError::CannotResetActiveGesture)
        );
        assert_eq!(session.state(), before);
        assert_eq!(
            session.pointer_down(payload, Point::ZERO),
            Err(DragSessionError::SessionNotIdle)
        );

        session.cancel().unwrap();
        assert_eq!(
            session.pointer_move(Point::ZERO),
            Err(DragSessionError::NoActiveGesture)
        );
        assert_eq!(
            session.pointer_down(payload, Point::ZERO),
            Err(DragSessionError::SessionNotIdle)
        );
        session.reset().unwrap();
        assert_eq!(session.state(), DragState::Idle);
    }
}
