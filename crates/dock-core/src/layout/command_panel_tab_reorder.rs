use super::*;

impl Layout {
    pub(super) fn try_reorder_panel_within_tabs(
        &mut self,
        panel: crate::PanelId,
        source_group: NodeId,
        source_index: usize,
        target_group: NodeId,
        target_index: usize,
    ) -> Result<Option<CommandApplication>, CommandError> {
        if source_group != target_group {
            return Ok(None);
        }
        if target_index == source_index || target_index == source_index + 1 {
            return Ok(Some(CommandApplication::persistent()));
        }

        let tabs = match self.nodes.get_mut(source_group) {
            Some(Node::Tabs(tabs)) => tabs,
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "panel parent is not a Tabs node",
                });
            }
        };
        let active_node = tabs.children.get(tabs.active_index).copied();
        let moved = tabs.children.remove(source_index);
        let insert_index = if target_index > source_index {
            target_index - 1
        } else {
            target_index
        };
        tabs.children
            .insert(insert_index.min(tabs.children.len()), moved);
        if let Some(active_node) = active_node
            && let Some(active_index) = tabs.children.iter().position(|id| *id == active_node)
        {
            tabs.active_index = active_index;
        }

        Ok(Some(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::ReorderTab {
                group: source_group,
                panel,
                to_index: source_index,
            }),
            persistent: true,
        }))
    }
}
