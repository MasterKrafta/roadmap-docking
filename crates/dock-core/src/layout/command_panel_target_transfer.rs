use super::*;

impl Layout {
    pub(super) fn move_panel_to_drop_target(
        &mut self,
        panel_node: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        self.remember_panel_dock_location(panel_node);
        self.detach_node(panel_node)?;
        self.insert_node_for_target(panel_node, target, metrics, Some(0.5))?;

        // In the Photoshop profile a center/tab drop is an explicit reveal action.
        // Same-group reorder/no-op paths return before reaching this helper.
        if self.profile == crate::Profile::Photoshop
            && let DropTarget::Tabs {
                group: target_group,
                ..
            } = target
            && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(target_group)
        {
            tabs.collapsed = false;
        }

        self.remember_panel_dock_location(panel_node);
        Ok(())
    }

    pub(super) fn move_panel_to_rail_slot(
        &mut self,
        panel: crate::PanelId,
        panel_node: NodeId,
        column: NodeId,
        index: usize,
        metrics: &DockMetrics,
        inverse: Option<DockCommand>,
    ) -> Result<CommandApplication, CommandError> {
        let order = self.column_display_order(column)?;
        if let Some(source_index) = order.iter().position(|item| item.panel == panel)
            && (index == source_index || index == source_index + 1)
        {
            return Ok(CommandApplication::persistent());
        }

        self.remember_panel_dock_location(panel_node);
        let moved = self.insert_panel_for_rail_slot(panel_node, column, index, metrics)?;
        if !moved {
            return Ok(CommandApplication::persistent());
        }
        self.remember_panel_dock_location(panel_node);
        Ok(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        })
    }
}
