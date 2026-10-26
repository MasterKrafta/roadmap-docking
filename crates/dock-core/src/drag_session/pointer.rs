use super::DragSession;
use crate::{
    DockMetrics, DragCancelReason, DragHandle, DragPayload, DragPreview, DragSessionError,
    DragState, DropTarget, Point,
};

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
            handle: None,
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
        self.pointer_down_with_handle(payload, DragHandle::default_for(payload), position)
    }

    /// Starts a gesture from an explicit interaction surface.
    ///
    /// In particular, adapters must use FlyoutPanel for panel content inside an open
    /// rail flyout, FloatingTitle for the title bar, and FloatingCapture for a dedicated
    /// dock/stack capture area. DragSession never mutates Layout, so the source flyout
    /// stays visible until the adapter applies a terminal drop command; cancellation
    /// emits no request and leaves it untouched.
    pub fn pointer_down_with_handle(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        position: Point,
    ) -> Result<(), DragSessionError> {
        if !matches!(self.state, DragState::Idle) {
            return Err(DragSessionError::SessionNotIdle);
        }
        if !handle.accepts(payload) {
            return Err(DragSessionError::PayloadHandleMismatch);
        }
        Self::validate_position(position)?;

        self.state = DragState::Pressed {
            payload,
            origin: position,
            position,
        };
        self.handle = Some(handle);
        Ok(())
    }

    /// Updates the pointer position and begins dragging once the Euclidean
    /// distance from the original press reaches the configured threshold.
    ///
    /// Moving after a target was resolved clears that target. The adapter must
    /// resolve the target again for the new point before release.
    pub fn pointer_move(&mut self, position: Point) -> Result<(), DragSessionError> {
        if !position.is_finite() {
            if matches!(self.state, DragState::Pressed { .. } | DragState::Dragging { .. }) {
                self.cancel_with_reason(DragCancelReason::InvalidPointerPosition)?;
            }
            return Err(DragSessionError::InvalidPointerPosition);
        }

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
    pub fn set_drop_target(&mut self, target: Option<DropTarget>) -> Result<(), DragSessionError> {
        if self.handle == Some(DragHandle::FloatingTitle) {
            return Err(DragSessionError::NotDockingDrag);
        }
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

    fn validate_position(position: Point) -> Result<(), DragSessionError> {
        if position.is_finite() {
            Ok(())
        } else {
            Err(DragSessionError::InvalidPointerPosition)
        }
    }

    pub(super) fn threshold_reached(origin: Point, position: Point, threshold: f32) -> bool {
        let dx = position.x - origin.x;
        let dy = position.y - origin.y;
        dx.hypot(dy) >= threshold
    }
}
