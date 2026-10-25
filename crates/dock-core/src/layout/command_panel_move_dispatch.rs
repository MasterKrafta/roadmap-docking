use super::*;

impl Layout {
    pub(super) fn apply_move_panel_command(
        &mut self,
        panel: &crate::PanelId,
        target: &DropTarget,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if matches!(target, DropTarget::GroupTabs { .. }) {
            return Err(CommandError::InvalidTarget {
                reason: "GroupTabs target is only valid when merging a whole Tabs group",
            });
        }
        let panel_node = self
            .find_panel_node(*panel)
            .ok_or(CommandError::MissingPanel { panel: *panel })?;
        self.validate_drop_target(*target, metrics)?;
        if let DropTarget::GroupEdge { group: target_group, .. } = *target
            && self.tab_parent_index(panel_node).is_some_and(|(source_group, _)| source_group == target_group)
        {
            return Ok(CommandApplication::persistent());
        }
        self.validate_panel_ids_allowed_for_target(&[*panel], *target)?;
        if let (
            Some((source_group, source_index)),
            DropTarget::Tabs {
                group: target_group,
                index,
            },
        ) = (self.tab_parent_index(panel_node), *target)
            && let Some(application) = self.try_reorder_panel_within_tabs(
                *panel,
                source_group,
                source_index,
                target_group,
                index,
            )?
        {
            return Ok(application);
        }
        let inverse = self.panel_transfer_inverse(*panel, panel_node);
        if let DropTarget::RailSlot { column, index } = *target {
            return self.move_panel_to_rail_slot(
                *panel,
                panel_node,
                column,
                index,
                metrics,
                inverse,
            );
        }
        self.move_panel_to_drop_target(panel_node, *target, metrics)?;
        Ok(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        })
    }
}
