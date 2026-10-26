use super::*;
use super::helpers::*;

pub(super) struct AdjacentColumnResize<'a> {
    pub(super) left_id: NodeId,
    pub(super) right_id: NodeId,
    pub(super) left: &'a Column,
    pub(super) right: &'a Column,
    pub(super) requested_delta: f32,
    pub(super) metrics: &'a DockMetrics,
    pub(super) width_error_reason: &'static str,
}

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
            AdjacentColumnResize {
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
            AdjacentColumnResize {
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

    /// Resizes two adjacent Columns using their combined min/max constraints.
    /// Main-dock and nested Split(X) callers share this invariant; each caller
    /// remains responsible for syncing its containing layout ratios.
    pub(super) fn resize_adjacent_columns(
        &mut self,
        request: AdjacentColumnResize<'_>,
    ) -> Result<f32, CommandError> {
        let AdjacentColumnResize {
            left_id,
            right_id,
            left,
            right,
            requested_delta,
            metrics,
            width_error_reason,
        } = request;
        let actual_delta = if left.mode.is_rail() || right.mode.is_rail() {
            0.0
        } else {
            let left_min = self.minimum_column_width(left_id, metrics)?;
            let right_min = self.minimum_column_width(right_id, metrics)?;
            let left_max = self.maximum_column_width(left_id)?.unwrap_or(f32::INFINITY);
            let right_max = self.maximum_column_width(right_id)?.unwrap_or(f32::INFINITY);
            let lower_delta = (left_min - left.width).max(right.width - right_max);
            let upper_delta = (right.width - right_min).min(left_max - left.width);
            if lower_delta > upper_delta {
                return Err(CommandError::InvalidValue {
                    field: "delta",
                    reason: "adjacent columns have incompatible min/max width constraints",
                });
            }
            requested_delta.clamp(lower_delta, upper_delta)
        };

        if actual_delta != 0.0 {
            let new_left = left.width + actual_delta;
            let new_right = right.width - actual_delta;
            if !new_left.is_finite()
                || !new_right.is_finite()
                || new_left <= 0.0
                || new_right <= 0.0
            {
                return Err(CommandError::InvalidValue {
                    field: "delta",
                    reason: width_error_reason,
                });
            }
            if let Some(Node::Column(column)) = self.nodes.get_mut(left_id) {
                column.width = new_left;
                column.last_expanded_width = Some(new_left);
            }
            if let Some(Node::Column(column)) = self.nodes.get_mut(right_id) {
                column.width = new_right;
                column.last_expanded_width = Some(new_right);
            }
        }

        Ok(actual_delta)
    }

    /// Computes width limits for ResizeColumn, including the main-dock canvas cap.
    pub(super) fn column_resize_limits(
        &self,
        column_id: NodeId,
        current: &Column,
        metrics: &DockMetrics,
    ) -> Result<(f32, f32), CommandError> {
        let main_position = self.main_column_position(column_id);
        if let Some((side, column_index)) = main_position {
            let main_columns = self.main_columns(side)?;
            let canvas_adjacent = match side {
                DockSide::Left => column_index + 1 == main_columns.len(),
                DockSide::Right => column_index == 0,
            };
            if !canvas_adjacent {
                return Err(CommandError::InvalidTarget {
                    reason: "interior main columns must be resized through ResizeColumnBoundary",
                });
            }
        }

        let min_width = self.minimum_column_width(column_id, metrics)?;
        let panel_max = self.maximum_column_width(column_id)?;
        let mut max_width = panel_max.unwrap_or(f32::MAX);

        // Resizing an outer main-dock column consumes or returns the measured
        // canvas remainder. Keep the required min_canvas_width available.
        if main_position.is_some()
            && let Some(bounds) = self.canvas.last_known_bounds
        {
            let room = (bounds.size.width - metrics.min_canvas_width).max(0.0);
            let canvas_cap = current.width + room;
            if canvas_cap.is_finite() {
                max_width = max_width.min(canvas_cap);
            }
        }
        if max_width < min_width {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "column minimum width exceeds its effective maximum width",
            });
        }
        Ok((min_width, max_width))
    }

    pub(super) fn adjust_canvas_for_column_resize(
        &mut self,
        column_id: NodeId,
        width_delta: f32,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let Some((side, _)) = self.main_column_position(column_id) else {
            return Ok(());
        };
        let Some(bounds) = self.canvas.last_known_bounds else {
            // The UI geometry pass will compute the canvas remainder once it
            // has a measured parent viewport.
            return Ok(());
        };
        let new_width = bounds.size.width - width_delta;
        let new_origin = Point::new(
            if side == DockSide::Left {
                bounds.origin.x + width_delta
            } else {
                bounds.origin.x
            },
            bounds.origin.y,
        );
        if !new_width.is_finite() || new_width + PIXEL_TOLERANCE < metrics.min_canvas_width {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "column resize would leave the canvas below min_canvas_width",
            });
        }
        let new_bounds = Rect::new(new_origin, Size::new(new_width, bounds.size.height));
        new_bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "canvas_bounds",
                reason: geometry_error_reason(error),
            })?;
        self.canvas.last_known_bounds = Some(new_bounds);
        Ok(())
    }

    pub(super) fn resize_inserted_column_for_split_ratio(
        &mut self,
        target_column: NodeId,
        new_column: NodeId,
        ratio: f32,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
            return Err(CommandError::InvalidValue {
                field: "ratio",
                reason: "split ratio must be between zero and one",
            });
        }
        let target_width = match self.nodes.get(target_column) {
            Some(Node::Column(column)) => column.width,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "split-ratio target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: target_column }),
        };
        let minimum = self.minimum_column_width(new_column, metrics)?;
        let maximum = self
            .maximum_column_width(new_column)?
            .unwrap_or(f32::MAX);
        if !target_width.is_finite()
            || target_width <= 0.0
            || !minimum.is_finite()
            || !maximum.is_finite()
            || maximum < minimum
        {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "split-ratio column width constraints are invalid",
            });
        }

        // Ratio describes the inserted column's share of the inserted/target
        // pair. Existing columns keep their widths; the new column grows or
        // shrinks to make the requested ratio visible, subject to its limits.
        let requested_width = target_width * ratio / (1.0 - ratio);
        if !requested_width.is_finite() || requested_width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "split-ratio column width overflowed",
            });
        }
        let width = requested_width.clamp(minimum, maximum);
        let Some(Node::Column(column)) = self.nodes.get_mut(new_column) else {
            return Err(CommandError::InvalidTarget {
                reason: "inserted main column changed unexpectedly",
            });
        };
        column.width = width;
        if !column.mode.is_rail() {
            column.last_expanded_width = Some(width);
        }
        Ok(())
    }
}
