use super::DragSession;
use crate::{
    DragCancelReason, DragDropRequest, DragHandle, DragPayload, DragSessionError, DragState,
    DropTarget, Point,
};

impl DragSession {
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
        if !position.is_finite() {
            if matches!(self.state, DragState::Pressed { .. } | DragState::Dragging { .. }) {
                self.cancel_with_reason(DragCancelReason::InvalidPointerPosition)?;
            }
            return Err(DragSessionError::InvalidPointerPosition);
        }

        let current_state = self.state;
        let handle = self.handle.unwrap_or_else(|| match current_state {
            DragState::Pressed { payload, .. } | DragState::Dragging { payload, .. } => {
                DragHandle::default_for(payload)
            }
            DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                DragHandle::PanelTab
            }
        });
        let request = match current_state {
            DragState::Pressed {
                payload, origin, ..
            } => {
                if !Self::threshold_reached(origin, position, self.threshold) {
                    self.state = DragState::Cancelled {
                        payload,
                        origin,
                        position,
                        reason: DragCancelReason::BelowThreshold,
                    };
                    return Ok(None);
                }

                Self::finish_release(
                    payload,
                    handle,
                    origin,
                    position,
                    resolved_target,
                    &mut self.state,
                )
            }
            DragState::Dragging {
                payload, origin, ..
            } => Self::finish_release(
                payload,
                handle,
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

    pub(super) fn cancel_with_reason(&mut self, reason: DragCancelReason) -> Result<(), DragSessionError> {
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
                self.handle = None;
                Ok(())
            }
        }
    }

    fn finish_release(
        payload: DragPayload,
        handle: DragHandle,
        origin: Point,
        position: Point,
        target: Option<DropTarget>,
        state: &mut DragState,
    ) -> Option<DragDropRequest> {
        // A title drag is a window move, not a docking operation, so it emits
        // a request even when the pointer is released over no DropTarget.
        let target = if handle == DragHandle::FloatingTitle {
            None
        } else {
            let Some(target) = target else {
                *state = DragState::Cancelled {
                    payload,
                    origin,
                    position,
                    reason: DragCancelReason::NoValidTarget,
                };
                return None;
            };
            Some(target)
        };

        *state = DragState::Dropped {
            payload,
            origin,
            position,
            target,
        };
        Some(DragDropRequest {
            payload,
            handle,
            origin,
            point: position,
            target,
            floating_bounds: None,
        })
    }
}
