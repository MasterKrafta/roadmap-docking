use super::*;
use super::helpers::normalized_ratios;

impl Layout {
    pub(super) fn dock_floating_group(
        &mut self,
        group_id: FloatingGroupId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let floating = self
            .floating_groups
            .get(group_id)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group: group_id })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?
            .root;
        self.validate_drop_target(target, metrics)?;
        if target.referenced_node_id().is_some_and(|node| self.is_descendant_or_same(root, node)) {
            return Ok(());
        }
        if let DropTarget::Tabs { .. } | DropTarget::GroupTabs { .. } = target {
            if matches!(self.nodes.get(root), Some(Node::Split(split)) if split.axis == Axis::Y) {
                return Err(CommandError::InvalidTarget {
                    reason: "a floating stack cannot be merged into a tab strip; target a dock column or rail slot",
                });
            }
        }
        match (self.nodes.get(root).cloned(), target) {
            (Some(Node::Split(split)), DropTarget::MainColumn { side, index })
                if split.axis == Axis::Y =>
            {
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
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: split.children,
                    group_ratios: normalized_ratios(split.ratios.into_iter().map(Some).collect()),
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(side, index, column, None, metrics)
            }
            (Some(Node::Split(split)), DropTarget::RailSlot { column, index })
                if split.axis == Axis::Y =>
            {
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
            (
                Some(Node::Tabs(_)),
                DropTarget::GroupTabs {
                    group: target_group,
                    index,
                },
            ) => self.merge_group_into_tabs(root, target_group, index),
            (Some(Node::Split(split)), DropTarget::ColumnSlot { column, index })
                if split.axis == Axis::Y =>
            {
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
            (
                Some(Node::Tabs(mut tabs)),
                DropTarget::Tabs {
                    group: target_group,
                    index,
                },
            ) => {
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
            (Some(Node::Split(_)), _) => Err(CommandError::InvalidTarget {
                reason: "floating split stacks can only be docked into a main column or icon rail",
            }),
            (Some(_), _) => {
                self.detach_node(root)?;
                self.insert_node_for_target(root, target, metrics, Some(0.5))
            }
            (None, _) => Err(CommandError::MissingNode { node: root }),
        }
    }
}
