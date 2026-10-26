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

    pub(super) fn apply_reset_main_column_ratios_command(
        &mut self,
        side: &DockSide,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
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

    pub(super) fn effective_column_width(
        requested_width: f32,
        mode: ColumnMode,
        min_width: f32,
        max_width: f32,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let actual_width = match mode {
            ColumnMode::Expanded => requested_width.clamp(min_width, max_width),
            ColumnMode::IconsAndLabels => metrics.min_column_width,
            ColumnMode::IconsOnly => metrics.rail_width,
        };
        if !actual_width.is_finite() || actual_width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "effective width is not finite and positive",
            });
        }
        Ok(actual_width)
    }
}
