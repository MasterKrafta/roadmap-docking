use super::*;

impl Layout {
    pub(super) fn apply_resize_column_boundary_command(
        &mut self,
        side: &DockSide,
        boundary_index: &usize,
        delta: &f32,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !delta.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "delta",
                reason: "column boundary delta must be finite",
            });
        }
        let columns = self.main_columns(*side)?;
        if *boundary_index >= columns.len().saturating_sub(1) {
            return Err(CommandError::InvalidTarget {
                reason: "column boundary index does not identify adjacent columns",
            });
        }
        let left_id = columns[*boundary_index];
        let right_id = columns[*boundary_index + 1];
        let left = match self.nodes.get(left_id) {
            Some(Node::Column(column)) => column.clone(),
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "main dock contains a non-Column child",
                });
            }
        };
        let right = match self.nodes.get(right_id) {
            Some(Node::Column(column)) => column.clone(),
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "main dock contains a non-Column child",
                });
            }
        };
        let actual_delta = self.resize_adjacent_columns(
            super::column_resize_helpers::AdjacentColumnResize {
                left_id,
                right_id,
                left: &left,
                right: &right,
                requested_delta: *delta,
                metrics,
                width_error_reason: "resized column width is not finite and positive",
            },
        )?;
        if actual_delta != 0.0 {
            self.sync_main_column_ratios(left_id)?;
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::ColumnBoundaryDelta(actual_delta)),
            inverse: (actual_delta != 0.0).then_some(DockCommand::ResizeColumnBoundary {
                side: *side,
                boundary_index: *boundary_index,
                delta: -actual_delta,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_resize_split_column_boundary_command(
        &mut self,
        split: &NodeId,
        boundary_index: &usize,
        delta: &f32,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !delta.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "delta",
                reason: "split column boundary delta must be finite",
            });
        }
        let current_split = match self.nodes.get(*split) {
            Some(Node::Split(value)) if value.axis == Axis::X => value.clone(),
            Some(Node::Split(_)) => {
                return Err(CommandError::InvalidTarget {
                    reason: "ResizeSplitColumnBoundary target must be a horizontal Split(X)",
                });
            }
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "ResizeSplitColumnBoundary target must be a Split node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *split }),
        };
        if current_split.children.len() < 2
            || current_split
                .children
                .iter()
                .any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_))))
        {
            return Err(CommandError::InvalidTarget {
                reason: "ResizeSplitColumnBoundary requires a Split(X) containing only Columns",
            });
        }
        if *boundary_index >= current_split.children.len() - 1 {
            return Err(CommandError::InvalidTarget {
                reason: "split column boundary index does not identify adjacent columns",
            });
        }

        let left_id = current_split.children[*boundary_index];
        let right_id = current_split.children[*boundary_index + 1];
        let left = match self.nodes.get(left_id) {
            Some(Node::Column(column)) => column.clone(),
            _ => return Err(CommandError::InvalidTarget {
                reason: "split column boundary references a non-Column node",
            }),
        };
        let right = match self.nodes.get(right_id) {
            Some(Node::Column(column)) => column.clone(),
            _ => return Err(CommandError::InvalidTarget {
                reason: "split column boundary references a non-Column node",
            }),
        };

        let actual_delta = self.resize_adjacent_columns(
            super::column_resize_helpers::AdjacentColumnResize {
                left_id,
                right_id,
                left: &left,
                right: &right,
                requested_delta: *delta,
                metrics,
                width_error_reason: "resized split column width is not finite and positive",
            },
        )?;
        if actual_delta != 0.0 {
            // Split(X) of Columns is laid out from each Column's requested
            // width. Keep ratios in sync so the tree remains meaningful if
            // another layout mode later consumes the stored ratios.
            let widths: Vec<f32> = current_split
                .children
                .iter()
                .map(|child_id| match self.nodes.get(*child_id) {
                    Some(Node::Column(column)) => Ok(column.width),
                    _ => Err(CommandError::InvalidTarget {
                        reason: "split column boundary references a non-Column node",
                    }),
                })
                .collect::<Result<_, _>>()?;
            let total_width = widths.iter().sum::<f32>();
            if !total_width.is_finite() || total_width <= 0.0 {
                return Err(CommandError::InvalidValue {
                    field: "delta",
                    reason: "split column widths must have a finite positive total",
                });
            }
            if let Some(Node::Split(split_node)) = self.nodes.get_mut(*split) {
                for (ratio, width) in split_node.ratios.iter_mut().zip(widths) {
                    *ratio = width / total_width;
                }
            }
        }

        Ok(CommandApplication {
            actual_value: Some(CommandValue::ColumnBoundaryDelta(actual_delta)),
            inverse: (actual_delta != 0.0).then_some(
                DockCommand::ResizeSplitColumnBoundary {
                    split: *split,
                    boundary_index: *boundary_index,
                    delta: -actual_delta,
                },
            ),
            persistent: true,
        })
    }
}
