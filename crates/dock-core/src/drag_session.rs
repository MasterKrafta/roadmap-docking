use crate::{DockMetrics, DragPayload, DragSessionError, DragState, Point};

/// A deterministic, UI-independent drag gesture state machine.
///
/// The threshold is copied from `DockMetrics::drag_threshold` at creation,
/// so it is configurable and stable for the lifetime of this gesture session.
/// Pointer positions and the threshold are measured in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragSession {
    state: DragState,
    threshold: f32,
}

impl DragSession {
    /// Creates an idle session using the configured logical-pixel drag
    /// threshold. Only the threshold is consumed; callers may validate the
    /// complete metrics object at their normal layout boundary.
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
    /// Repeated updates while dragging only update the latest pointer position.
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
                    }
                } else {
                    DragState::Pressed {
                        payload,
                        origin,
                        position,
                    }
                }
            }
            DragState::Dragging { payload, origin, .. } => DragState::Dragging {
                payload,
                origin,
                position,
            },
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                return Err(DragSessionError::NoActiveGesture);
            }
        };
        Ok(())
    }

    /// Releases the pointer. A release below the threshold is a cancellation
    /// rather than a drop, so a normal click does not become a drag. The
    /// release position is processed too, in case no final move event arrived.
    pub fn pointer_up(&mut self, position: Point) -> Result<(), DragSessionError> {
        self.pointer_move(position)?;

        self.state = match self.state {
            DragState::Pressed {
                payload,
                origin,
                position,
            } => DragState::Cancelled {
                payload,
                origin,
                position,
            },
            DragState::Dragging {
                payload,
                origin,
                position,
            } => DragState::Dropped {
                payload,
                origin,
                position,
            },
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                return Err(DragSessionError::NoActiveGesture);
            }
        };
        Ok(())
    }

    /// Cancels a pressed or dragging gesture while preserving its last known
    /// pointer position and payload in the terminal state.
    pub fn cancel(&mut self) -> Result<(), DragSessionError> {
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
            } => DragState::Cancelled {
                payload,
                origin,
                position,
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
        DockMetrics, DragPayload, DragSessionError, DragState, PanelId, Point,
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

        session
            .pointer_up(Point::new(13.5, 20.0))
            .unwrap();
        assert_eq!(
            session.state(),
            DragState::Cancelled {
                payload,
                origin: Point::new(10.0, 20.0),
                position: Point::new(13.5, 20.0),
            }
        );
    }

    #[test]
    fn four_logical_pixels_starts_dragging_and_release_drops_payload() {
        let payload = panel_payload();
        let mut session = session();

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_move(Point::new(4.0, 0.0)).unwrap();
        assert_eq!(
            session.state(),
            DragState::Dragging {
                payload,
                origin: Point::ZERO,
                position: Point::new(4.0, 0.0),
            }
        );

        session.pointer_move(Point::new(8.0, 2.0)).unwrap();
        session.pointer_up(Point::new(10.0, 2.0)).unwrap();
        assert_eq!(
            session.state(),
            DragState::Dropped {
                payload,
                origin: Point::ZERO,
                position: Point::new(10.0, 2.0),
            }
        );
    }

    #[test]
    fn release_position_can_cross_threshold_without_an_intermediate_move_event() {
        let payload = panel_payload();
        let mut session = session();

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.pointer_up(Point::new(0.0, 4.0)).unwrap();

        assert_eq!(
            session.state(),
            DragState::Dropped {
                payload,
                origin: Point::ZERO,
                position: Point::new(0.0, 4.0),
            }
        );
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
    }

    #[test]
    fn cancellation_works_before_and_after_drag_and_reset_reuses_session() {
        let payload = panel_payload();
        let mut session = session();

        session.pointer_down(payload, Point::ZERO).unwrap();
        session.cancel().unwrap();
        assert!(matches!(session.state(), DragState::Cancelled { .. }));

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
    fn invalid_positions_threshold_and_transitions_are_rejected_without_mutation() {
        let mut metrics = DockMetrics::default();
        metrics.drag_threshold = f32::NAN;
        assert_eq!(
            DragSession::new(&metrics),
            Err(DragSessionError::InvalidThreshold)
        );

        let mut session = session();
        assert_eq!(
            session.pointer_down(
                panel_payload(),
                Point::new(f32::NAN, 0.0)
            ),
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
