use super::*;

impl Layout {
    pub(super) fn apply_column_resize_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::ResizeColumn { column, width } => {
                self.apply_resize_column_command(column, width, metrics)
            }
            DockCommand::ResizeColumnBoundary {
                side,
                boundary_index,
                delta,
            } => {
                self.apply_resize_column_boundary_command(side, boundary_index, delta, metrics)
            }
            DockCommand::ResizeSplitColumnBoundary {
                split,
                boundary_index,
                delta,
            } => {
                self.apply_resize_split_column_boundary_command(split, boundary_index, delta, metrics)
            }
            DockCommand::ResetMainColumnRatios { side } => {
                self.apply_reset_main_column_ratios_command(side, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "column resize dispatcher received an unsupported command",
            }),
        }
    }
}
