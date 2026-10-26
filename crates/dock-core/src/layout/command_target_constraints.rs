use super::*;
use crate::AllowedArea;

impl Layout {
    pub(super) fn validate_panel_ids_allowed_for_area(
        &self,
        panels: &[PanelId],
        area: AllowedArea,
    ) -> Result<(), CommandError> {
        for panel in panels {
            let instance = self
                .panel_instance(*panel)
                .ok_or(CommandError::MissingPanel { panel: *panel })?;
            if !instance.constraints.allows_area(area) {
                return Err(CommandError::InvalidTarget {
                    reason: super::command_target_area::allowed_area_rejection_reason(area),
                });
            }
        }
        Ok(())
    }

    pub(super) fn validate_constraints_allowed_for_target(
        &self,
        constraints: crate::PanelConstraints,
        target: DropTarget,
    ) -> Result<(), CommandError> {
        let area = self.allowed_area_for_target(target)?;
        if constraints.allows_area(area) {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget {
                reason: super::command_target_area::allowed_area_rejection_reason(area),
            })
        }
    }

    pub(super) fn validate_panel_ids_allowed_for_target(
        &self,
        panels: &[PanelId],
        target: DropTarget,
    ) -> Result<(), CommandError> {
        let area = self.allowed_area_for_target(target)?;
        self.validate_panel_ids_allowed_for_area(panels, area)
    }
}
