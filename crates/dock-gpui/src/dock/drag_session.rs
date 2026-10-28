use std::{collections::HashSet, fmt};

use dock_core::{
    ComputeRectsError, DockMetrics, DragHandle, DragPayload, DragSession as CoreDragSession,
    DragSessionError, DropZoneResolver, Point, Rect,
};

use crate::requests::DragOverlay;

/// Geometry contract captured when a drag resolver is prepared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DragGeometrySnapshot {
    pub(super) layout_version: u64,
    pub(super) viewport: Rect,
    pub(super) metrics: DockMetrics,
}

impl DragGeometrySnapshot {
    pub(super) fn matches(self, layout_version: u64, viewport: Rect, metrics: DockMetrics) -> bool {
        self.layout_version == layout_version
            && self.viewport == viewport
            && self.metrics == metrics
    }
}

/// A pointer-down that may become a drag after the user crosses the drag threshold.
///
/// The Core state machine and resolver are allocated only when the threshold is crossed,
/// so a normal click does not create an active drag session.
pub(crate) struct DragGesture {
    pub(super) payload: DragPayload,
    pub(super) handle: DragHandle,
    /// External drags resolve against the main dock only, not hidden floating geometry.
    pub(super) external_window_drag: bool,
    pub(super) origin: Point,
    pub(super) core: Option<CoreDragSession>,
    pub(super) snapshot: Option<DragGeometrySnapshot>,
    pub(super) resolver: Option<DropZoneResolver>,
    /// Nodes belonging to the floating window being captured; targets inside it are not docking targets.
    pub(super) capture_root_nodes: Option<HashSet<dock_core::NodeId>>,
    /// Whole-column capture can only dock into a main-column slot.
    pub(super) capture_main_column_only: bool,
    pub(super) overlay: Option<DragOverlay>,
}

impl fmt::Debug for DragGesture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DragGesture")
            .field("payload", &self.payload)
            .field("handle", &self.handle)
            .field("external_window_drag", &self.external_window_drag)
            .field("origin", &self.origin)
            .field("core_started", &self.core.is_some())
            .field("snapshot", &self.snapshot)
            .field("resolver_prepared", &self.resolver.is_some())
            .field(
                "capture_root_node_count",
                &self.capture_root_nodes.as_ref().map(|nodes| nodes.len()),
            )
            .field(
                "capture_main_column_only",
                &self.capture_main_column_only,
            )
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

impl fmt::Display for DragUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => write!(formatter, "core drag update failed: {error:?}"),
            Self::Geometry(error) => {
                write!(
                    formatter,
                    "drop-zone geometry preparation failed: {error:?}"
                )
            }
            Self::MissingFrameGeometry => {
                formatter.write_str("drag activation requires current frame geometry")
            }
            Self::StaleFrameGeometry => formatter.write_str("drag frame geometry is stale"),
        }
    }
}

impl std::error::Error for DragUpdateError {}

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
            external_window_drag: false,
            origin,
            core: None,
            snapshot: None,
            resolver: None,
            capture_root_nodes: None,
            capture_main_column_only: false,
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
        assert!(gesture.snapshot.is_none());
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
