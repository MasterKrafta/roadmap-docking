use super::*;

impl Layout {
    pub(super) fn dock_floating_stack_to_rail_slot(
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
        let insert_at = self.rail_slot_group_index(column, index)?;
        self.detach_node(root)?;
        self.nodes.remove(root);
        for (insert_at, child) in (insert_at..).zip(split.children) {
            self.insert_group_at_column(column, child, insert_at, None)?;
        }
        Ok(())
    }
}
