use super::*;
use super::helpers::geometry_error_reason;

impl Layout {
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
