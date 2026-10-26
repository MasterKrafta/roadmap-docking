use super::helpers::*;
use super::*;

impl Layout {
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

    pub(super) fn reset_main_column_widths(
        &mut self,
        side: DockSide,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let columns = self.main_columns(side)?;
        if columns.len() < 2 {
            return Ok(());
        }

        let current_widths = self.main_column_widths_for_reset(&columns)?;
        let total_width: f32 = current_widths.iter().copied().sum();
        if !total_width.is_finite() || total_width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "total dock column width must be finite and positive",
            });
        }

        let (minimums, maximums) = self.main_column_width_constraints_for_reset(
            &columns,
            &current_widths,
            total_width,
            metrics,
        )?;
        let widths = Self::equalized_main_column_widths(total_width, &minimums, &maximums)?;

        for (id, width) in columns.into_iter().zip(widths) {
            if let Some(Node::Column(column)) = self.nodes.get_mut(id)
                && !column.mode.is_rail()
            {
                column.width = width;
                column.last_expanded_width = Some(width);
            }
        }
        Ok(())
    }

    fn main_column_widths_for_reset(
        &self,
        columns: &[NodeId],
    ) -> Result<Vec<f32>, CommandError> {
        columns
            .iter()
            .map(|id| match self.nodes.get(*id) {
                Some(Node::Column(column)) => Ok(column.width),
                _ => Err(CommandError::InvalidTarget {
                    reason: "main horizontal split contains a non-Column child",
                }),
            })
            .collect()
    }

    fn main_column_width_constraints_for_reset(
        &self,
        columns: &[NodeId],
        current_widths: &[f32],
        total_width: f32,
        metrics: &DockMetrics,
    ) -> Result<(Vec<f32>, Vec<f32>), CommandError> {
        if columns.len() != current_widths.len() {
            return Err(CommandError::InvalidTarget {
                reason: "main horizontal split contains a non-Column child",
            });
        }

        // Rails stay fixed at their presentation width. Expanded columns are
        // equalized within the minimum/maximum constraints of their contents.
        let mut minimums = Vec::with_capacity(columns.len());
        let mut maximums = Vec::with_capacity(columns.len());
        for (id, current_width) in columns.iter().copied().zip(current_widths) {
            let column = match self.nodes.get(id) {
                Some(Node::Column(column)) => column,
                _ => {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split contains a non-Column child",
                    });
                }
            };
            if column.mode.is_rail() {
                minimums.push(*current_width);
                maximums.push(*current_width);
            } else {
                let minimum = self.minimum_column_width(id, metrics)?;
                let maximum = self
                    .maximum_column_width(id)?
                    .unwrap_or(total_width)
                    .min(total_width);
                if !minimum.is_finite() || !maximum.is_finite() || maximum < minimum {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "column width constraints are invalid",
                    });
                }
                minimums.push(minimum);
                maximums.push(maximum);
            }
        }

        let minimum_total: f32 = minimums.iter().copied().sum();
        let maximum_total: f32 = maximums.iter().copied().sum();
        if total_width + 1.0e-3 < minimum_total || total_width > maximum_total + 1.0e-3 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "current dock width cannot satisfy column min/max constraints",
            });
        }
        Ok((minimums, maximums))
    }

    fn equalized_main_column_widths(
        total_width: f32,
        minimums: &[f32],
        maximums: &[f32],
    ) -> Result<Vec<f32>, CommandError> {
        if minimums.len() != maximums.len() {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "column width constraints are invalid",
            });
        }

        // Sum(clamp(target, minimum, maximum)) is monotonic, so bisection finds
        // the equal-width target even when some columns hit a constraint.
        let mut low = 0.0_f32;
        let mut high = total_width;
        for _ in 0..48 {
            let target = low + (high - low) * 0.5;
            let allocated: f32 = minimums
                .iter()
                .zip(maximums)
                .map(|(minimum, maximum)| target.clamp(*minimum, *maximum))
                .sum();
            if allocated < total_width {
                low = target;
            } else {
                high = target;
            }
        }

        let target = low + (high - low) * 0.5;
        let mut widths: Vec<f32> = minimums
            .iter()
            .zip(maximums)
            .map(|(minimum, maximum)| target.clamp(*minimum, *maximum))
            .collect();
        let mut residual = total_width - widths.iter().copied().sum::<f32>();
        for index in 0..widths.len() {
            if residual.abs() <= 1.0e-3 {
                break;
            }
            let adjustment = if residual > 0.0 {
                residual.min(maximums[index] - widths[index])
            } else {
                residual.max(minimums[index] - widths[index])
            };
            widths[index] += adjustment;
            residual -= adjustment;
        }
        if residual.abs() > 1.0e-2 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "equalized column widths do not preserve total dock width",
            });
        }
        Ok(widths)
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

    pub(super) fn next_floating_z_order(&self) -> Result<i32, CommandError> {
        match self
            .floating_groups
            .values()
            .map(|group| group.z_order)
            .max()
        {
            Some(value) => value.checked_add(1).ok_or(CommandError::InvalidValue {
                field: "z_order",
                reason: "floating z-order overflow",
            }),
            None => Ok(0),
        }
    }
}
