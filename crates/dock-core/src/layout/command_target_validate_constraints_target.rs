use super::*;

impl Layout {
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
                reason: super::command_target_area_rejection_reason::allowed_area_rejection_reason(area),
            })
        }
    }
}
