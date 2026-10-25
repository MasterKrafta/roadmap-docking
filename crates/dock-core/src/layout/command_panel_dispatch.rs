use super::*;

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
            } => self.apply_open_panel_command(key, constraints, target, metrics),
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
