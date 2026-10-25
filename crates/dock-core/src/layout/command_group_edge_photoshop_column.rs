use super::*;

impl Layout {
    pub(super) fn insert_group_edge_in_photoshop_column(
        &mut self,
        new_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        split_ratio: f32,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        match zone {
            DropZone::Top | DropZone::Bottom => {
                let groups = match self.nodes.get(column_id) {
                    Some(Node::Column(column)) => column.groups.clone(),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "target group has no owning column",
                        });
                    }
                };
                let target_index = groups.iter().position(|id| *id == target_group).ok_or(
                    CommandError::InvalidTarget {
                        reason: "target group is not in its owning column",
                    },
                )?;
                let index = if zone == DropZone::Top {
                    target_index
                } else {
                    target_index + 1
                };
                self.insert_group_at_column(
                    column_id,
                    new_group,
                    index,
                    Some((split_ratio, target_index)),
                )
            }
            DropZone::Left | DropZone::Right => {
                let (side, column_index) = self.main_column_position(column_id).ok_or(
                    CommandError::InvalidTarget {
                        reason: "horizontal group splits require a column in a main dock",
                    },
                )?;
                let insert_at = if zone == DropZone::Left {
                    column_index
                } else {
                    column_index + 1
                };
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![new_group],
                    group_ratios: vec![1.0],
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(
                    side,
                    insert_at,
                    column,
                    Some((split_ratio, column_index)),
                    metrics,
                )
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group-edge target requires a cardinal edge zone",
            }),
        }
    }
}
