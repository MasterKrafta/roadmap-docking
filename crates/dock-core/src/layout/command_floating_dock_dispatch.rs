use super::*;

impl Layout {
    pub(super) fn apply_dock_floating_command(
        &mut self,
        group: &FloatingGroupId,
        target: &DropTarget,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        let root = self
            .floating_groups
            .get(*group)
            .and_then(|floating| self.dock_areas.get(floating.area).map(|area| area.root))
            .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
        self.validate_drop_target(*target, metrics)?;
        let panel_nodes = self.panel_nodes_in_subtree(root);
        let moving_panels = self.panel_ids_in_subtree(root);
        self.validate_panel_ids_allowed_for_target(&moving_panels, *target)?;
        self.dock_floating_group(*group, *target, metrics)?;
        for panel_node in panel_nodes {
            self.remember_panel_dock_location(panel_node);
        }
        Ok(CommandApplication::persistent())
    }
}
