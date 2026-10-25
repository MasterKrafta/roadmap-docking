use super::*;
use super::helpers::geometry_error_reason;

impl Layout {
    pub(super) fn apply_panel_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::OpenPanel {
                key,
                constraints,
                target,
            } => {
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
            DockCommand::ClosePanel { panel } => self.apply_close_panel_command(panel),
            DockCommand::CloseGroup { group } => self.apply_close_group_command(group),
            DockCommand::ActivateTab { group, panel } => {
                self.apply_activate_tab_command(group, panel)
            }
            DockCommand::ReorderTab {
                group,
                panel,
                to_index,
            } => self.apply_reorder_tab_command(group, panel, to_index),
            _ => Err(CommandError::InvalidTarget {
                reason: "apply panel command received an unsupported command",
            }),
        }
    }
}
