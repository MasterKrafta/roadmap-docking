use dock_core::{DragHandle, DragPayload, Point};

/// A pointer-down that may become a drag after the user crosses the drag threshold.
///
/// This initial state deliberately does not create the Core DragSession: a normal
/// click on a tab must remain a click if the pointer is released before the threshold.
#[derive(Debug)]
pub(crate) struct DragGesture {
    pub(super) payload: DragPayload,
    pub(super) handle: DragHandle,
    pub(super) origin: Point,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DragStartError {
    SessionAlreadyActive,
    InvalidPointer,
    PayloadHandleMismatch,
}

impl DragGesture {
    pub(super) fn begin(
        payload: DragPayload,
        handle: DragHandle,
        origin: Point,
    ) -> Result<Self, DragStartError> {
        if !origin.is_finite() {
            return Err(DragStartError::InvalidPointer);
        }
        if !handle.accepts(payload) {
            return Err(DragStartError::PayloadHandleMismatch);
        }

        Ok(Self {
            payload,
            handle,
            origin,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DragGesture, DragStartError};
    use dock_core::{DragHandle, DragPayload, NodeId, Point};
    use slotmap::Key;

    #[test]
    fn pending_drag_preserves_payload_handle_and_origin() {
        let payload = DragPayload::Group {
            group: NodeId::null(),
        };
        let origin = Point::new(24.0, 36.0);
        let gesture = DragGesture::begin(payload, DragHandle::GroupGrip, origin).unwrap();

        assert_eq!(gesture.payload, payload);
        assert_eq!(gesture.handle, DragHandle::GroupGrip);
        assert_eq!(gesture.origin, origin);
    }

    #[test]
    fn pending_drag_rejects_non_finite_origin() {
        assert_eq!(
            DragGesture::begin(
                DragPayload::Group {
                    group: NodeId::null(),
                },
                DragHandle::GroupGrip,
                Point::new(f32::NAN, 0.0),
            )
            .unwrap_err(),
            DragStartError::InvalidPointer,
        );
    }

    #[test]
    fn pending_drag_rejects_mismatched_payload_and_handle() {
        assert_eq!(
            DragGesture::begin(
                DragPayload::Group {
                    group: NodeId::null(),
                },
                DragHandle::PanelTab,
                Point::ZERO,
            )
            .unwrap_err(),
            DragStartError::PayloadHandleMismatch,
        );
    }
}
