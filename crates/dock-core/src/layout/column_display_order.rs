use crate::{ColumnDisplayItem, CommandError, Layout, Node, NodeId};

impl Layout {
    /// Returns the canonical flattened panel order for a column.
    ///
    /// Entries follow the column's group/tree order from top to bottom, then tab
    /// order within each Tabs group from left to right. Rail modes use this order
    /// for their visible items. The query is read-only and is also available
    /// before a mode switch, allowing adapters to preserve stable identity.
    pub fn column_display_order(
        &self,
        column_id: NodeId,
    ) -> Result<Vec<ColumnDisplayItem>, CommandError> {
        let column = match self.node(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "column display order requires a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };

        let mut output = Vec::new();
        for group_id in &column.groups {
            self.append_column_display_items(*group_id, &mut output)?;
        }
        Ok(output)
    }

    fn append_column_display_items(
        &self,
        node_id: NodeId,
        output: &mut Vec<ColumnDisplayItem>,
    ) -> Result<(), CommandError> {
        match self.node(node_id) {
            Some(Node::Tabs(tabs)) => {
                for (tab_index, child_id) in tabs.children.iter().copied().enumerate() {
                    let panel_id = match self.node(child_id) {
                        Some(Node::Panel(panel)) => panel.panel_id,
                        Some(_) => {
                            return Err(CommandError::InvalidTarget {
                                reason: "column display order requires Panel leaves inside Tabs",
                            });
                        }
                        None => return Err(CommandError::MissingNode { node: child_id }),
                    };
                    output.push(ColumnDisplayItem::new(
                        node_id,
                        panel_id,
                        tab_index,
                        output.len(),
                    ));
                }
                Ok(())
            }
            Some(Node::Split(split)) => {
                for child in &split.children {
                    self.append_column_display_items(*child, output)?;
                }
                Ok(())
            }
            Some(Node::Column(column)) => {
                for group in &column.groups {
                    self.append_column_display_items(*group, output)?;
                }
                Ok(())
            }
            Some(Node::Panel(_)) => Err(CommandError::InvalidTarget {
                reason: "column display order requires Tabs groups rather than loose Panel nodes",
            }),
            None => Err(CommandError::MissingNode { node: node_id }),
        }
    }
}
