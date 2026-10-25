use super::*;

impl Layout {
    pub(super) fn apply_panel_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MovePanelFromFlyout { .. }
            | DockCommand::FloatPanelFromFlyout { .. } => {
                self.apply_flyout_panel_transfer_command(command, metrics)
            }
            DockCommand::MovePanel { panel, target } => {
                self.apply_move_panel_command(panel, target, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "apply panel transfer command received an unsupported command",
            }),
        }
    }
}
