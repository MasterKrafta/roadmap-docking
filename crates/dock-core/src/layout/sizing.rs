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
