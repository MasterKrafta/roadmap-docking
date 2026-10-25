use super::*;

impl Layout {
    pub(super) fn dock_floating_stack_to_column_slot(
        &mut self,
        root: NodeId,
        split: Split,
        column: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        if split
            .children
            .iter()
            .any(|id| !matches!(self.nodes.get(*id), Some(Node::Tabs(_))))
        {
            return Err(CommandError::InvalidTarget {
                reason: "floating stack contains a non-Tabs group",
            });
        }
        self.detach_node(root)?;
        self.nodes.remove(root);
        for (insert_at, child) in (index..).zip(split.children) {
            self.insert_group_at_column(column, child, insert_at, None)?;
        }
        Ok(())
    }
}
