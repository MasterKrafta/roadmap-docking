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
}
