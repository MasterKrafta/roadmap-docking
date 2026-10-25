use super::*;

impl Layout {
    pub(super) fn apply_close_panel_command(
        &mut self,
        panel: &crate::PanelId,
    ) -> Result<CommandApplication, CommandError> {
        if !self.panel_instances.contains_key(*panel) {
            return Err(CommandError::MissingPanel { panel: *panel });
        }
        let node = self
            .find_panel_node(*panel)
            .ok_or(CommandError::MissingPanel { panel: *panel })?;
        self.remember_panel_dock_location(node);
        self.detach_node(node)?;
        self.remove_subtree(node);
        Ok(CommandApplication::persistent())
    }

    pub(super) fn apply_close_group_command(
        &mut self,
        group: &NodeId,
    ) -> Result<CommandApplication, CommandError> {
        match self.nodes.get(*group) {
            Some(Node::Tabs(_)) => {}
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "CloseGroup target must be a Tabs node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *group }),
        }
        self.remember_subtree_dock_locations(*group);
        self.detach_node(*group)?;
        self.remove_subtree(*group);
        Ok(CommandApplication::persistent())
    }
}
