use super::*;

impl Layout {
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
}
