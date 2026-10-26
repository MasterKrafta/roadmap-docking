use super::*;
use crate::{DragDropAction, DragDropRequest, DragHandle, DragPayload};

impl Layout {
    /// Resolves a completed drag request to either one command or an explicit
    /// no-op. It only inspects pure model state; no GPUI pointer/window types
    /// participate and the Layout is never mutated.
    pub fn resolve_drag_drop(
        &self,
        request: &DragDropRequest,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
        metrics.validate().map_err(|_| CommandError::InvalidValue {
            field: "metrics",
            reason: "dock metrics are invalid",
        })?;
        self.validate(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;
        if !request.origin.is_finite() || !request.point.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "pointer",
                reason: "drag pointer coordinates must be finite",
            });
        }
        if !request.handle.accepts(request.payload) {
            return Err(CommandError::InvalidTarget {
                reason: "drag handle does not match the payload kind",
            });
        }

        match (request.handle, request.payload) {
            (DragHandle::FloatingTitle, DragPayload::FloatingGroup { group }) => {
                self.resolve_floating_title_drag(request, group)
            }
            (DragHandle::FloatingCapture, DragPayload::FloatingGroup { group }) => {
                self.resolve_floating_capture_drag(request, group, metrics)
            }
            (DragHandle::PanelTab | DragHandle::FlyoutPanel, DragPayload::Panel { panel }) => {
                self.resolve_panel_drag(request, panel, metrics)
            }
            (DragHandle::GroupGrip, DragPayload::Group { group }) => {
                self.resolve_group_grip_drag(request, group, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "drag handle and payload do not describe a supported drag operation",
            }),
        }
    }
}
