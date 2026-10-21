use crate::{ColumnDisplayItem, CommandError, Layout, Node, NodeId};

impl Layout {
    /// Returns the canonical flattened panel order for a column.
    ///
    /// Entries follow group order from top to bottom, then tab order within each
    /// group from left to right. Rail modes use this order for their visible
    /// items. The query is read-only and is also available before a mode switch,
    /// allowing adapters to preserve stable identity across presentations.
    pub fn column_display_order(
        &self,
        column_id: NodeId,
    ) -> Result<Vec<ColumnDisplayItem>, CommandError> {
        let groups = match self.node(column_id) {
            Some(Node::Column(column)) => &column.groups,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "column display order requires a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };

        let mut output = Vec::new();
        for group_id in groups {
            let tabs = match self.node(*group_id) {
                Some(Node::Tabs(tabs)) => tabs,
                Some(_) => {
                    return Err(CommandError::InvalidTarget {
                        reason: "column display order requires Tabs groups",
                    });
                }
                None => return Err(CommandError::MissingNode { node: *group_id }),
            };

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
                    *group_id,
                    panel_id,
                    tab_index,
                    output.len(),
                ));
            }
        }
        Ok(output)
    }
}
