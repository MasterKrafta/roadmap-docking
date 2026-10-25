use super::*;

impl Layout {
    pub(super) fn apply_group_resize_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::ResizeGroups {
                column,
                boundary_index,
                delta,
                available_height,
            } => {
                self.apply_resize_groups_command(
                    column,
                    boundary_index,
                    delta,
                    available_height,
                    metrics,
                )
            }

            DockCommand::ResetColumnRatios {
                column,
                available_height,
            } => {
                self.apply_reset_column_ratios_command(column, available_height, metrics)
            }
            DockCommand::RestoreGroupRatios { column, ratios } => {
                self.apply_restore_group_ratios_command(column, ratios)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group resize dispatcher received an unsupported command",
            }),
        }
    }
}
