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
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                let bounds = Rect::new(*origin, floating.bounds.size);
                bounds
                    .validate_non_empty()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "origin",
                        reason: geometry_error_reason(error),
                    })?;
                if floating.bounds.origin == *origin {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds.origin = *origin;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::MoveFloating {
                        group: *group,
                        origin: floating.bounds.origin,
                    }),
                    persistent: true,
                })
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
            DockCommand::RaiseFloating { group } => {
                let current = self
                    .floating_groups
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                let max_other = self
                    .floating_groups
                    .iter()
                    .filter(|(other_id, _)| *other_id != *group)
                    .map(|(_, value)| value.z_order)
                    .max();
                if max_other.is_none_or(|max| current.z_order > max) {
                    return Ok(CommandApplication::persistent());
                }
                let max_z = self
                    .floating_groups
                    .iter()
                    .map(|(_, value)| value.z_order)
                    .max()
                    .unwrap_or(-1);
                let next_z = max_z.checked_add(1).ok_or(CommandError::InvalidValue {
                    field: "z_order",
                    reason: "floating z-order overflow",
                })?;
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.z_order = next_z;
                }
                Ok(CommandApplication::persistent())
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "floating window dispatcher received a non-floating-window command",
            }),
        }
    }
}
