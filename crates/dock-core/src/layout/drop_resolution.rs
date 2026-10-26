use super::*;
use crate::{
    AllowedArea, DragDropAction, DragDropRequest, DragHandle, DragPayload, FloatingGroupId, NodeId,
};

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

    pub(super) fn resolve_floating_title_drag(
        &self,
        request: &DragDropRequest,
        group: FloatingGroupId,
    ) -> Result<DragDropAction, CommandError> {
        if request.target.is_some() {
            return Err(CommandError::InvalidTarget {
                reason: "floating title drag moves the window and does not use a dock target",
            });
        }
        let floating = self
            .floating_group(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let delta_x = request.point.x - request.origin.x;
        let delta_y = request.point.y - request.origin.y;
        let origin = floating
            .bounds
            .origin
            .checked_offset(delta_x, delta_y)
            .map_err(|_| CommandError::InvalidValue {
                field: "origin",
                reason: "floating title drag produced invalid window coordinates",
            })?;
        if origin == floating.bounds.origin {
            return Ok(DragDropAction::NoOp);
        }
        Ok(DragDropAction::Command(DockCommand::MoveFloating {
            group,
            origin,
        }))
    }

    pub(super) fn resolve_floating_capture_drag(
        &self,
        request: &DragDropRequest,
        group: FloatingGroupId,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
        let target = request.target.ok_or(CommandError::InvalidTarget {
            reason: "floating capture drag requires a resolved dock/stack target",
        })?;
        if matches!(target, DropTarget::FreeArea) {
            return Err(CommandError::InvalidTarget {
                reason: "a floating capture drag requires a dock/stack target",
            });
        }
        self.validate_drop_target(target, metrics)?;
        let root = self
            .floating_group(group)
            .and_then(|floating| self.dock_area(floating.area))
            .map(|area| area.root)
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        if target
            .referenced_node_id()
            .is_some_and(|node| self.is_descendant_or_same(root, node))
        {
            return Ok(DragDropAction::NoOp);
        }
        let moving_panels = self.panel_ids_in_subtree(root);
        self.validate_panel_ids_allowed_for_target(&moving_panels, target)?;
        Ok(DragDropAction::Command(DockCommand::DockFloating {
            group,
            target,
        }))
    }
}
