use super::*;

impl Layout {
    /// Resizes two adjacent Columns using their combined min/max constraints.
    /// Main-dock and nested Split(X) callers share this invariant; each caller
    /// remains responsible for syncing its containing layout ratios.
    pub(super) fn resize_adjacent_columns(
        &mut self,
        left_id: NodeId,
        right_id: NodeId,
        left: &Column,
        right: &Column,
        requested_delta: f32,
        metrics: &DockMetrics,
        width_error_reason: &'static str,
    ) -> Result<f32, CommandError> {
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


}
