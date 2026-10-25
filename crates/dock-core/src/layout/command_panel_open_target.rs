use super::*;
use crate::PanelConstraints;

impl Layout {
    pub(super) fn resolve_and_validate_open_panel_target(
        &self,
        key: &PanelKey,
        target: Option<DropTarget>,
        constraints: PanelConstraints,
        metrics: &DockMetrics,
    ) -> Result<DropTarget, CommandError> {
        let resolved_target = match target {
            Some(target) => target,
            None => self.resolve_open_target(key, metrics)?,
        };
        self.validate_drop_target(resolved_target, metrics)?;
        self.validate_constraints_allowed_for_target(constraints, resolved_target)?;
        Ok(resolved_target)
    }
}
