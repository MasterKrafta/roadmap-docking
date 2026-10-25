use super::*;

impl Layout {
    pub(super) fn apply_group_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MoveGroup { group, column, index } => {
                self.apply_move_group_command(group, column, index)
            }
            DockCommand::MoveGroupToMainColumn { group, side, index } => {
                self.apply_move_group_to_main_column_command(group, side, index, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group transfer dispatcher received an unsupported command",
            }),
        }
    }
}
