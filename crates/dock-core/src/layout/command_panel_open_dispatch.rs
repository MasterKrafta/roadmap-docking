use super::*;
use super::helpers::geometry_error_reason;

impl Layout {
    pub(super) fn apply_open_panel_command(
        &mut self,
        key: &PanelKey,
        constraints: &PanelConstraints,
        target: &Option<DropTarget>,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        constraints
            .validate()
            .map_err(|error| CommandError::InvalidValue {
                field: "constraints",
                reason: geometry_error_reason(error),
            })?;

        if let Some(application) =
            self.try_reopen_existing_singleton_panel(key, constraints.singleton)?
        {
            return Ok(application);
        }

        let resolved_target = self.resolve_and_validate_open_panel_target(
            key,
            *target,
            *constraints,
            metrics,
        )?;
        self.insert_new_panel_for_open_target(
            key,
            *constraints,
            resolved_target,
            metrics,
        )
    }
}
