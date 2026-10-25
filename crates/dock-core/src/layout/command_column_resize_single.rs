use super::*;

impl Layout {
    pub(super) fn apply_resize_column_command(
        &mut self,
        column: &NodeId,
        width: &f32,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !width.is_finite() || *width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "width must be finite and positive",
            });
        }
        let current = match self.nodes.get(*column) {
            Some(Node::Column(column)) => column.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "ResizeColumn target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *column }),
        };
        let old_width = current.width;
        let (min_width, max_width) =
            self.column_resize_limits(*column, &current, metrics)?;
        let actual_width = Self::effective_column_width(
            *width,
            current.mode,
            min_width,
            max_width,
            metrics,
        )?;
        let actual_delta = actual_width - old_width;
        if actual_delta != 0.0 {
            if let Some(Node::Column(column)) = self.nodes.get_mut(*column) {
                column.width = actual_width;
                if current.mode == ColumnMode::Expanded {
                    column.last_expanded_width = Some(actual_width);
                }
            }
            self.adjust_canvas_for_column_resize(*column, actual_delta, metrics)?;
            self.sync_main_column_ratios(*column)?;
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::ColumnWidth(actual_width)),
            inverse: (actual_delta != 0.0).then_some(DockCommand::ResizeColumn {
                column: *column,
                width: old_width,
            }),
            persistent: true,
        })
    }
}
