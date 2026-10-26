use super::*;

impl Layout {
    pub(super) fn is_group_in_icon_rail(&self, group: NodeId) -> bool {
        let Some(column_id) = self.find_owner_column(group) else {
            return false;
        };
        self.main_column_position(column_id).is_some()
            && matches!(self.nodes.get(column_id), Some(Node::Column(column)) if column.mode.is_rail())
    }

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

    pub(super) fn insert_panel_for_rail_slot(
        &mut self,
        panel_node: NodeId,
        column_id: NodeId,
        index: usize,
        metrics: &DockMetrics,
    ) -> Result<bool, CommandError> {
        self.validate_drop_target(
            DropTarget::RailSlot {
                column: column_id,
                index,
            },
            metrics,
        )?;
        let panel_id = match self.nodes.get(panel_node) {
            Some(Node::Panel(panel)) => panel.panel_id,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "rail icon insertion accepts a Panel node",
                });
            }
            None => return Err(CommandError::MissingNode { node: panel_node }),
        };
        let order = self.column_display_order(column_id)?;
        if index > order.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "rail insertion index is outside the flattened icon order",
            });
        }
        let source_parent = self.tab_parent_index(panel_node);
        let source_group = source_parent.map(|(group, _)| group);
        let source_display_index = order.iter().position(|item| item.panel == panel_id);
        if source_display_index.is_some_and(|source| index == source || index == source + 1) {
            return Ok(false);
        }

        // Between icons of one Tabs group means reorder; crossing its boundary
        // means placing the Panel in a new Tabs group at that column position.
        let target_tabs =
            if index > 0 && index < order.len() && order[index - 1].group == order[index].group {
                Some((order[index].group, order[index].tab_index))
            } else if index == 0
                && source_group.is_some()
                && order
                    .first()
                    .is_some_and(|item| Some(item.group) == source_group)
            {
                Some((order[0].group, 0))
            } else if index == order.len()
                && source_group.is_some()
                && order
                    .last()
                    .is_some_and(|item| Some(item.group) == source_group)
            {
                let group =
                    order
                        .last()
                        .map(|item| item.group)
                        .ok_or(CommandError::InvalidTarget {
                            reason: "cannot resolve the final icon group",
                        })?;
                let len = match self.nodes.get(group) {
                    Some(Node::Tabs(tabs)) => tabs.children.len(),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "rail icon group must be a Tabs node",
                        });
                    }
                };
                Some((group, len))
            } else {
                None
            };

        if let Some((target_group, target_index)) = target_tabs {
            let target_len = match self.nodes.get(target_group) {
                Some(Node::Tabs(tabs)) => tabs.children.len(),
                Some(_) => {
                    return Err(CommandError::InvalidTarget {
                        reason: "rail icon insertion target must be a Tabs node",
                    });
                }
                None => return Err(CommandError::MissingNode { node: target_group }),
            };
            if target_index > target_len {
                return Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "rail tab insertion index is outside the target group",
                });
            }
            if source_group == Some(target_group) {
                let (_, source_index) = source_parent.ok_or(CommandError::InvalidTarget {
                    reason: "rail reorder source has no Tabs parent",
                })?;
                let (active_node, children) = match self.nodes.get(target_group) {
                    Some(Node::Tabs(tabs)) => (
                        tabs.children.get(tabs.active_index).copied(),
                        tabs.children.clone(),
                    ),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "rail reorder parent is not a Tabs node",
                        });
                    }
                };
                let mut reordered = children;
                let moved = reordered.remove(source_index);
                let insert_at = if target_index > source_index {
                    target_index - 1
                } else {
                    target_index
                };
                reordered.insert(insert_at.min(reordered.len()), moved);
                let active_index = active_node
                    .and_then(|active| reordered.iter().position(|node| *node == active))
                    .unwrap_or(insert_at.min(reordered.len().saturating_sub(1)));
                if let Some(Node::Tabs(tabs)) = self.nodes.get_mut(target_group) {
                    tabs.children = reordered;
                    tabs.active_index = active_index;
                }
                return Ok(true);
            }

            if self.find_parent(panel_node).is_some()
                || self.dock_areas.values().any(|area| area.root == panel_node)
            {
                self.detach_node(panel_node)?;
            }
            let Some(Node::Tabs(tabs)) = self.nodes.get_mut(target_group) else {
                return Err(CommandError::InvalidTarget {
                    reason: "rail insertion target changed unexpectedly",
                });
            };
            let insert_at = target_index.min(tabs.children.len());
            tabs.children.insert(insert_at, panel_node);
            tabs.active_index = insert_at;
            if self.profile == Profile::Photoshop {
                tabs.collapsed = false;
            }
            return Ok(true);
        }

        let group_index = self.rail_slot_group_index(column_id, index)?;
        if self.find_parent(panel_node).is_some()
            || self.dock_areas.values().any(|area| area.root == panel_node)
        {
            self.detach_node(panel_node)?;
        }
        let group = self.group_node_for(panel_node)?;
        self.insert_group_at_column(column_id, group, group_index, None)?;
        Ok(true)
    }
}
