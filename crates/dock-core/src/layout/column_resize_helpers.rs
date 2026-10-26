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
