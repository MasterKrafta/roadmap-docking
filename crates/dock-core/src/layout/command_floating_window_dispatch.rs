use super::*;
use super::helpers::geometry_error_reason;

impl Layout {
    pub(super) fn apply_floating_window_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MoveFloating { group, origin } => {
                self.apply_move_floating_command(*group, *origin)
            }
            DockCommand::ResizeFloating { group, size } => {
                size.validate_positive()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "size",
                        reason: geometry_error_reason(error),
                    })?;
                let floating = self
                    .floating_groups
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
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
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds = new_bounds;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::FloatingSize(actual)),
                    inverse: Some(DockCommand::ResizeFloating {
                        group: *group,
                        size: floating.bounds.size,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeFloatingBounds { group, bounds } => {
                bounds.validate_non_empty().map_err(|error| CommandError::InvalidValue {
                    field: "bounds",
                    reason: geometry_error_reason(error),
                })?;
                let floating = self
                    .floating_groups
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
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
                actual_bounds.validate_non_empty().map_err(|error| {
                    CommandError::InvalidValue {
                        field: "bounds",
                        reason: geometry_error_reason(error),
                    }
                })?;
                if actual_bounds == floating.bounds {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds = actual_bounds;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::ResizeFloatingBounds {
                        group: *group,
                        bounds: floating.bounds,
                    }),
                    persistent: true,
                })
            }
            DockCommand::RaiseFloating { group } => self.apply_raise_floating_command(*group),
            _ => Err(CommandError::InvalidTarget {
                reason: "floating window dispatcher received a non-floating-window command",
            }),
        }
    }
}
