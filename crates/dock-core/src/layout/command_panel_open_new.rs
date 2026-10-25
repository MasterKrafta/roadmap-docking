use super::*;

impl Layout {
    pub(super) fn insert_new_panel_for_open_target(
        &mut self,
        key: &PanelKey,
        constraints: PanelConstraints,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        let panel_id = self
            .panel_instances
            .insert(PanelInstance::new(key.clone(), constraints));
        let panel_node = self.nodes.insert(Node::Panel(crate::Panel::new(panel_id)));
        if let DropTarget::RailSlot { column, index } = target {
            self.insert_panel_for_rail_slot(panel_node, column, index, metrics)?;
        } else {
            self.insert_node_for_target(panel_node, target, metrics, Some(0.5))?;
            if self.profile == Profile::Photoshop
                && let DropTarget::Tabs { group, .. } = target
                && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group)
            {
                tabs.collapsed = false;
            }
        }
        self.remember_panel_dock_location(panel_node);
        Ok(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::ClosePanel { panel: panel_id }),
            persistent: true,
        })
    }
}
