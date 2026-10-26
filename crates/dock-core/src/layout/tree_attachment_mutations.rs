use super::*;

impl Layout {
    pub(super) fn replace_child(
        &mut self,
        parent: NodeId,
        old_child: NodeId,
        new_child: NodeId,
    ) -> Result<(), CommandError> {
        let Some(parent_node) = self.nodes.get_mut(parent) else {
            return Err(CommandError::MissingNode { node: parent });
        };
        let replaced = match parent_node {
            Node::Split(split) => split
                .children
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Tabs(tabs) => tabs
                .children
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Column(column) => column
                .groups
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Panel(_) => false,
        };
        if replaced {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget {
                reason: "target node is not a child of the expected parent",
            })
        }
    }

    pub(super) fn set_area_root(
        &mut self,
        old_root: NodeId,
        new_root: NodeId,
    ) -> Result<(), CommandError> {
        if let Some(area) = self
            .dock_areas
            .values_mut()
            .find(|area| area.root == old_root)
        {
            area.root = new_root;
            return Ok(());
        }
        Err(CommandError::InvalidTarget {
            reason: "target node is not a registered dock-area root",
        })
    }

    pub(super) fn detach_node(&mut self, node_id: NodeId) -> Result<(), CommandError> {
        if !self.nodes.contains_key(node_id) {
            return Err(CommandError::MissingNode { node: node_id });
        }
        if let Some(parent) = self.find_parent(node_id) {
            let Some(parent_node) = self.nodes.get_mut(parent) else {
                return Err(CommandError::MissingNode { node: parent });
            };
            match parent_node {
                Node::Split(split) => {
                    if let Some(index) = split.children.iter().position(|id| *id == node_id) {
                        split.children.remove(index);
                        if index < split.ratios.len() {
                            split.ratios.remove(index);
                        }
                    }
                }
                Node::Tabs(tabs) => {
                    let active_node = tabs.children.get(tabs.active_index).copied();
                    if let Some(index) = tabs.children.iter().position(|id| *id == node_id) {
                        tabs.children.remove(index);
                        if index < tabs.children.len() {
                            if active_node == Some(node_id) {
                                tabs.active_index = index;
                            } else if let Some(active_node) = active_node {
                                tabs.active_index = tabs
                                    .children
                                    .iter()
                                    .position(|id| *id == active_node)
                                    .unwrap_or(index);
                            } else {
                                tabs.active_index =
                                    index.min(tabs.children.len().saturating_sub(1));
                            }
                        } else {
                            tabs.active_index = tabs.children.len().saturating_sub(1);
                        }
                    }
                }
                Node::Column(column) => {
                    if let Some(index) = column.groups.iter().position(|id| *id == node_id) {
                        column.groups.remove(index);
                        if index < column.group_ratios.len() {
                            column.group_ratios.remove(index);
                        }
                    }
                }
                Node::Panel(_) => {
                    return Err(CommandError::InvalidTarget {
                        reason: "Panel leaves cannot own child nodes",
                    });
                }
            }
            return Ok(());
        }

        for index in 0..self.main_areas.len() {
            if let Some(area_id) = self.main_areas[index]
                && self
                    .dock_areas
                    .get(area_id)
                    .is_some_and(|area| area.root == node_id)
            {
                self.main_areas[index] = None;
                self.dock_areas.remove(area_id);
                return Ok(());
            }
        }
        let floating = self.floating_groups.iter().find_map(|(id, group)| {
            self.dock_areas
                .get(group.area)
                .is_some_and(|area| area.root == node_id)
                .then_some((id, group.area))
        });
        if let Some((group_id, area_id)) = floating {
            self.floating_groups.remove(group_id);
            self.dock_areas.remove(area_id);
            return Ok(());
        }
        Err(CommandError::InvalidTarget {
            reason: "node is not attached to a registered dock area or parent",
        })
    }

    pub(super) fn remove_subtree(&mut self, node_id: NodeId) {
        let Some(node) = self.nodes.get(node_id).cloned() else {
            return;
        };
        for child in node.children().iter().copied() {
            self.remove_subtree(child);
        }
        if let Node::Panel(panel) = node {
            self.panel_instances.remove(panel.panel_id);
        }
        self.nodes.remove(node_id);
    }

}
