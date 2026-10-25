use super::*;

impl Layout {
    pub(super) fn insert_group_edge(
        &mut self,
        new_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        ratio: Option<f32>,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if new_group == target_group {
            return Ok(());
        }
        if !matches!(self.nodes.get(new_group), Some(Node::Tabs(_)))
            || !matches!(self.nodes.get(target_group), Some(Node::Tabs(_)))
        {
            return Err(CommandError::InvalidTarget {
                reason: "group-edge operations require Tabs nodes",
            });
        }
        let split_ratio = ratio.unwrap_or(0.5);
        if !split_ratio.is_finite() || split_ratio <= 0.0 || split_ratio >= 1.0 {
            return Err(CommandError::InvalidValue {
                field: "ratio",
                reason: "split ratio must be between zero and one",
            });
        }

        if self.profile == Profile::Photoshop {
            if let Some(column_id) = self.find_owner_column(target_group) {
                self.insert_group_edge_in_photoshop_column(
                    new_group,
                    target_group,
                    zone,
                    split_ratio,
                    column_id,
                    metrics,
                )
            } else {
                self.insert_group_edge_in_floating_stack(
                    new_group,
                    target_group,
                    zone,
                    split_ratio,
                )
            }
        } else {
            self.insert_group_edge_generic(
                new_group,
                target_group,
                zone,
                split_ratio,
            )
        }
    }
}
