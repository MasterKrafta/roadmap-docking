use super::*;

impl Layout {
    pub(super) fn merge_group_into_tabs(
        &mut self,
        group: NodeId,
        target_group: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        if group == target_group {
            return match self.nodes.get(group) {
                Some(Node::Tabs(_)) => Ok(()),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "MergeGroup source must be a Tabs node",
                }),
                None => Err(CommandError::MissingNode { node: group }),
            };
        }
        let (source_children, source_active) = match self.nodes.get(group) {
            Some(Node::Tabs(tabs)) if !tabs.children.is_empty() => {
                (tabs.children.clone(), tabs.active_index)
            }
            Some(Node::Tabs(_)) => {
                return Err(CommandError::InvalidTarget {
                    reason: "cannot merge an empty source group",
                });
            }
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "MergeGroup source must be a Tabs node",
                });
            }
            None => return Err(CommandError::MissingNode { node: group }),
        };
        let target_len = match self.nodes.get(target_group) {
            Some(Node::Tabs(tabs)) => tabs.children.len(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "MergeGroup target must be a Tabs node",
                });
            }
            None => return Err(CommandError::MissingNode { node: target_group }),
        };
        if index > target_len {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "group-tab insertion index is outside the target group",
            });
        }
        if self.is_descendant_or_same(group, target_group)
            || self.is_descendant_or_same(target_group, group)
        {
            return Err(CommandError::InvalidTarget {
                reason: "cannot merge ancestor or descendant Tabs groups",
            });
        }

        self.detach_node(group)?;
        if let Some(Node::Tabs(source)) = self.nodes.get_mut(group) {
            source.children.clear();
            source.active_index = 0;
        }
        self.nodes.remove(group);

        let Some(Node::Tabs(target)) = self.nodes.get_mut(target_group) else {
            return Err(CommandError::InvalidTarget {
                reason: "MergeGroup target changed unexpectedly",
            });
        };
        for (offset, child) in source_children.iter().copied().enumerate() {
            target.children.insert(index + offset, child);
        }
        target.active_index = index + source_active.min(source_children.len() - 1);
        Ok(())
    }

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
