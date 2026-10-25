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
            } => self.apply_resize_column_boundary_command(side, boundary_index, delta, metrics),
            DockCommand::ResizeSplitColumnBoundary {
                split,
                boundary_index,
                delta,
            } => self.apply_resize_split_column_boundary_command(split, boundary_index, delta, metrics),
            DockCommand::ResetMainColumnRatios { side } => {
                let Some(area) = self.main_area(*side).copied() else {
                    return Ok(CommandApplication::persistent());
                };
                let root = self.nodes.get(area.root).cloned();
                match root {
                    Some(Node::Column(_)) => Ok(CommandApplication::persistent()),
                    Some(Node::Split(split)) if split.axis == Axis::X => {
                        if split.children.iter().any(|child| {
                            !matches!(self.nodes.get(*child), Some(Node::Column(_)))
                        }) {
                            return Err(CommandError::InvalidTarget {
                                reason: "main horizontal split contains a non-Column child",
                            });
                        }
                        let first_column = split.children.first().copied().ok_or(
                            CommandError::InvalidTarget {
                                reason: "main horizontal split has no columns",
                            },
                        )?;
                        self.reset_main_column_widths(*side, metrics)?;
                        self.sync_main_column_ratios(first_column)?;
                        Ok(CommandApplication::persistent())
                    }
                    Some(_) => Err(CommandError::InvalidTarget {
                        reason: "main dock root is not a horizontal column split",
                    }),
                    None => Err(CommandError::InvalidTarget {
                        reason: "main dock root is missing",
                    }),
                }
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "column resize dispatcher received an unsupported command",
            }),
        }
    }
}
