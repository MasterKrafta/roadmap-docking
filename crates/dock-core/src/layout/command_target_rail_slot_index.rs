use super::*;

impl Layout {
    pub(super) fn rail_slot_group_index(
        &self,
        column_id: NodeId,
        index: usize,
    ) -> Result<usize, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) if column.mode.is_rail() => column,
            Some(Node::Column(_)) => {
                return Err(CommandError::InvalidTarget {
                    reason: "rail group insertion requires a rail-mode column",
                });
            }
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "rail group insertion target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        let order = self.column_display_order(column_id)?;
        if index > order.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "rail insertion index is outside the flattened icon order",
            });
        }
        if index == order.len() {
            return Ok(column.groups.len());
        }
        let target_group = order[index].group;
        column
            .groups
            .iter()
            .position(|root| self.is_descendant_or_same(*root, target_group))
            .ok_or(CommandError::InvalidTarget {
                reason: "rail icon group is not attached to the target column",
            })
    }
}
