use super::*;

impl Layout {
    pub(super) fn dock_floating_tabs_to_tabs(
        &mut self,
        root: NodeId,
        mut tabs: Tabs,
        target_group: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        self.detach_node(root)?;
        if let Some(Node::Tabs(old_root)) = self.nodes.get_mut(root) {
            old_root.children.clear();
            old_root.active_index = 0;
        }
        self.nodes.remove(root);
        if let Some(Node::Tabs(target_tabs)) = self.nodes.get_mut(target_group) {
            let insert_at = index.min(target_tabs.children.len());
            let source_active_index = tabs.active_index;
            let moving_count = tabs.children.len();
            for (offset, child) in tabs.children.drain(..).enumerate() {
                target_tabs.children.insert(insert_at + offset, child);
            }
            if moving_count > 0 {
                target_tabs.active_index =
                    insert_at + source_active_index.min(moving_count - 1);
            }
        } else {
            return Err(CommandError::InvalidTarget {
                reason: "tab target changed unexpectedly",
            });
        }
        Ok(())
    }
}
