use super::*;
use crate::{DragDropAction, DragDropRequest, FloatingGroupId};

impl Layout {
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
