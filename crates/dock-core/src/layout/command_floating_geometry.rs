use super::*;
use super::helpers::geometry_error_reason;

impl Layout {
    pub(super) fn apply_move_floating_command(
        &mut self,
        group: FloatingGroupId,
        origin: Point,
    ) -> Result<CommandApplication, CommandError> {
        // Do not clamp to a monitor/work-area here: the adapter applies
        // its visible-area policy; Core stores finite logical coordinates.
        if !origin.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "origin",
                reason: "origin coordinates must be finite",
            });
        }
        let floating = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let bounds = Rect::new(origin, floating.bounds.size);
        bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "origin",
                reason: geometry_error_reason(error),
            })?;
        if floating.bounds.origin == origin {
            return Ok(CommandApplication::persistent());
        }
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.bounds.origin = origin;
        }
        Ok(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::MoveFloating {
                group,
                origin: floating.bounds.origin,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_resize_floating_command(
        &mut self,
        group: FloatingGroupId,
        size: Size,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        size.validate_positive()
            .map_err(|error| CommandError::InvalidValue {
                field: "size",
                reason: geometry_error_reason(error),
            })?;
        let floating = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?
            .root;
        let minimum = self.minimum_floating_size(root, metrics)?;
        let actual = Size::new(
            size.width.max(minimum.width),
            size.height.max(minimum.height),
        );
        let new_bounds = Rect::new(floating.bounds.origin, actual);
        new_bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "size",
                reason: geometry_error_reason(error),
            })?;
        if actual == floating.bounds.size {
            return Ok(CommandApplication {
                actual_value: Some(CommandValue::FloatingSize(actual)),
                inverse: None,
                persistent: true,
            });
        }
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.bounds = new_bounds;
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::FloatingSize(actual)),
            inverse: Some(DockCommand::ResizeFloating {
                group,
                size: floating.bounds.size,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_resize_floating_bounds_command(
        &mut self,
        group: FloatingGroupId,
        bounds: Rect,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "bounds",
                reason: geometry_error_reason(error),
            })?;
        let floating = self
            .floating_groups
            .get(group)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?
            .root;
        let minimum = self.minimum_floating_size(root, metrics)?;
        let actual_bounds = Rect::new(
            bounds.origin,
            Size::new(
                bounds.size.width.max(minimum.width),
                bounds.size.height.max(minimum.height),
            ),
        );
        actual_bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "bounds",
                reason: geometry_error_reason(error),
            })?;
        if actual_bounds == floating.bounds {
            return Ok(CommandApplication::persistent());
        }
        if let Some(value) = self.floating_groups.get_mut(group) {
            value.bounds = actual_bounds;
        }
        Ok(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::ResizeFloatingBounds {
                group,
                bounds: floating.bounds,
            }),
            persistent: true,
        })
    }
}
