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
                    reason: super::command_target_policy::allowed_area_rejection_reason(area),
                });
            }
        }
        Ok(())
    }
}
