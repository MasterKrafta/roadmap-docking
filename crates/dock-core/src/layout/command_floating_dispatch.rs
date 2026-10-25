use super::*;

impl Layout {
    pub(super) fn apply_floating_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::Float { node, bounds } => self.apply_float_command(node, bounds),
            DockCommand::FloatGroupAsColumn { group, bounds } => {
                self.apply_float_group_as_column_command(*group, *bounds, metrics)
            }
            DockCommand::FloatPanelAsGroup { panel, bounds } => {
                self.apply_float_panel_as_group_command(*panel, *bounds, metrics)
            }
            DockCommand::MoveFloating { .. }
            | DockCommand::ResizeFloating { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::RaiseFloating { .. } => {
                self.apply_floating_window_command(command, metrics)
            }
            DockCommand::DockFloating { group, target } => {
                self.apply_dock_floating_command(group, target, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "floating command dispatcher received a non-floating command",
            }),
        }
    }
}
