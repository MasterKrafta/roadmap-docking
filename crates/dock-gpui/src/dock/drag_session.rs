use std::fmt;

use dock_core::{
    ComputeRectsError, DragHandle, DragPayload, DragSession as CoreDragSession,
    DragSessionError, DropZoneResolver, Point,
};

use crate::requests::DragOverlay;

/// A pointer-down that may become a drag after the user crosses the drag threshold.
///
/// The Core state machine and resolver are allocated only when the threshold is crossed,
/// so a normal click does not create an active drag session.
pub(crate) struct DragGesture {
    pub(super) payload: DragPayload,
    pub(super) handle: DragHandle,
    pub(super) origin: Point,
    pub(super) core: Option<CoreDragSession>,
    pub(super) resolver: Option<DropZoneResolver>,
    pub(super) overlay: Option<DragOverlay>,
}

impl fmt::Debug for DragGesture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DragGesture")
            .field("payload", &self.payload)
            .field("handle", &self.handle)
            .field("origin", &self.origin)
            .field("core_started", &self.core.is_some())
            .field("resolver_prepared", &self.resolver.is_some())
            .field("overlay_available", &self.overlay.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DragStartError {
    SessionAlreadyActive,
    InvalidPointer,
    PayloadHandleMismatch,
}

#[derive(Debug)]
pub(crate) enum DragUpdateError {
    Core(DragSessionError),
    Geometry(ComputeRectsError),
    MissingFrameGeometry,
    StaleFrameGeometry,
}

impl From<DragSessionError> for DragUpdateError {
    fn from(error: DragSessionError) -> Self {
        Self::Core(error)
    }
}

impl From<ComputeRectsError> for DragUpdateError {
    fn from(error: ComputeRectsError) -> Self {
        Self::Geometry(error)
    }
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
            core: None,
            resolver: None,
            overlay: None,
        })
    }

    /// Uses the Euclidean threshold configured in DockMetrics; no UI constant is introduced.
    pub(super) fn threshold_reached(origin: Point, position: Point, threshold: f32) -> bool {
        if !position.is_finite() {
            return true;
        }

        let dx = position.x - origin.x;
        let dy = position.y - origin.y;
        dx.hypot(dy) >= threshold
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
        assert!(gesture.core.is_none());
        assert!(gesture.resolver.is_none());
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

    #[test]
    fn threshold_uses_euclidean_distance_and_includes_the_boundary() {
        let origin = Point::ZERO;

        assert!(!DragGesture::threshold_reached(
            origin,
            Point::new(2.0, 2.0),
            4.0,
        ));
        assert!(DragGesture::threshold_reached(
            origin,
            Point::new(3.0, 3.0),
            4.0,
        ));
        assert!(DragGesture::threshold_reached(
            origin,
            Point::new(4.0, 0.0),
            4.0,
        ));
    }
}
