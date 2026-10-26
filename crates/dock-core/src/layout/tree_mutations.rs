use super::*;

impl Layout {
    pub(super) fn find_owner_column(&self, group: NodeId) -> Option<NodeId> {
        self.nodes.iter().find_map(|(id, node)| match node {
            Node::Column(column) if column.groups.contains(&group) => Some(id),
            _ => None,
        })
    }

    pub(super) fn rail_column_for_group(&self, group_id: NodeId) -> Option<NodeId> {
        let main_roots = [DockSide::Left, DockSide::Right]
            .into_iter()
            .filter_map(|side| self.main_area(side).map(|area| area.root))
            .collect::<Vec<_>>();
        self.nodes.iter().find_map(|(column_id, node)| match node {
            Node::Column(column)
                if column.mode.is_rail()
                    && main_roots
                        .iter()
                        .any(|root| self.is_descendant_or_same(*root, column_id))
                    && column
                        .groups
                        .iter()
                        .any(|root| self.is_descendant_or_same(*root, group_id)) =>
            {
                Some(column_id)
            }
            _ => None,
        })
    }

    pub(super) fn main_column_position(&self, column_id: NodeId) -> Option<(DockSide, usize)> {
        for side in [DockSide::Left, DockSide::Right] {
            if let Ok(columns) = self.main_columns(side)
                && let Some(index) = columns.iter().position(|id| *id == column_id)
            {
                return Some((side, index));
            }
        }
        None
    }

    pub(super) fn find_floating_root_containing(&self, node_id: NodeId) -> Option<NodeId> {
        self.floating_groups.values().find_map(|group| {
            let root = self.dock_areas.get(group.area)?.root;
            self.is_descendant_or_same(root, node_id).then_some(root)
        })
    }

    pub(super) fn is_descendant_or_same(&self, root: NodeId, target: NodeId) -> bool {
        if root == target {
            return true;
        }
        let Some(node) = self.nodes.get(root) else {
            return false;
        };
        node.children()
            .iter()
            .copied()
            .any(|child| self.is_descendant_or_same(child, target))
    }

    pub(super) fn replace_child(
        &mut self,
        parent: NodeId,
        old_child: NodeId,
        new_child: NodeId,
    ) -> Result<(), CommandError> {
        let Some(parent_node) = self.nodes.get_mut(parent) else {
            return Err(CommandError::MissingNode { node: parent });
        };
        let replaced = match parent_node {
            Node::Split(split) => split
                .children
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Tabs(tabs) => tabs
                .children
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Column(column) => column
                .groups
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Panel(_) => false,
        };
        if replaced {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget {
                reason: "target node is not a child of the expected parent",
            })
        }
    }

    pub(super) fn set_area_root(
        &mut self,
        old_root: NodeId,
        new_root: NodeId,
    ) -> Result<(), CommandError> {
        if let Some(area) = self
            .dock_areas
            .values_mut()
            .find(|area| area.root == old_root)
        {
            area.root = new_root;
            return Ok(());
        }
        Err(CommandError::InvalidTarget {
            reason: "target node is not a registered dock-area root",
        })
    }

    pub(super) fn detach_node(&mut self, node_id: NodeId) -> Result<(), CommandError> {
        if !self.nodes.contains_key(node_id) {
            return Err(CommandError::MissingNode { node: node_id });
        }
        if let Some(parent) = self.find_parent(node_id) {
            let Some(parent_node) = self.nodes.get_mut(parent) else {
                return Err(CommandError::MissingNode { node: parent });
            };
            match parent_node {
                Node::Split(split) => {
                    if let Some(index) = split.children.iter().position(|id| *id == node_id) {
                        split.children.remove(index);
                        if index < split.ratios.len() {
                            split.ratios.remove(index);
                        }
                    }
                }
                Node::Tabs(tabs) => {
                    let active_node = tabs.children.get(tabs.active_index).copied();
                    if let Some(index) = tabs.children.iter().position(|id| *id == node_id) {
                        tabs.children.remove(index);
                        if index < tabs.children.len() {
                            if active_node == Some(node_id) {
                                tabs.active_index = index;
                            } else if let Some(active_node) = active_node {
                                tabs.active_index = tabs
                                    .children
                                    .iter()
                                    .position(|id| *id == active_node)
                                    .unwrap_or(index);
                            } else {
                                tabs.active_index =
                                    index.min(tabs.children.len().saturating_sub(1));
                            }
                        } else {
                            tabs.active_index = tabs.children.len().saturating_sub(1);
                        }
                    }
                }
                Node::Column(column) => {
                    if let Some(index) = column.groups.iter().position(|id| *id == node_id) {
                        column.groups.remove(index);
                        if index < column.group_ratios.len() {
                            column.group_ratios.remove(index);
                        }
                    }
                }
                Node::Panel(_) => {
                    return Err(CommandError::InvalidTarget {
                        reason: "Panel leaves cannot own child nodes",
                    });
                }
            }
            return Ok(());
        }

        for index in 0..self.main_areas.len() {
            if let Some(area_id) = self.main_areas[index]
                && self
                    .dock_areas
                    .get(area_id)
                    .is_some_and(|area| area.root == node_id)
            {
                self.main_areas[index] = None;
                self.dock_areas.remove(area_id);
                return Ok(());
            }
        }
        let floating = self.floating_groups.iter().find_map(|(id, group)| {
            self.dock_areas
                .get(group.area)
                .is_some_and(|area| area.root == node_id)
                .then_some((id, group.area))
        });
        if let Some((group_id, area_id)) = floating {
            self.floating_groups.remove(group_id);
            self.dock_areas.remove(area_id);
            return Ok(());
        }
        Err(CommandError::InvalidTarget {
            reason: "node is not attached to a registered dock area or parent",
        })
    }

    pub(super) fn remove_subtree(&mut self, node_id: NodeId) {
        let Some(node) = self.nodes.get(node_id).cloned() else {
            return;
        };
        for child in node.children().iter().copied() {
            self.remove_subtree(child);
        }
        if let Node::Panel(panel) = node {
            self.panel_instances.remove(panel.panel_id);
        }
        self.nodes.remove(node_id);
    }

    pub(super) fn sync_main_column_ratios(
        &mut self,
        column_id: NodeId,
    ) -> Result<(), CommandError> {
        let Some((side, _)) = self.main_column_position(column_id) else {
            return Ok(());
        };
        let columns = self.main_columns(side)?;
        if columns.len() < 2 {
            return Ok(());
        }
        let widths: Vec<f32> = columns
            .iter()
            .map(|id| match self.nodes.get(*id) {
                Some(Node::Column(column)) => Ok(column.width),
                _ => Err(CommandError::InvalidTarget {
                    reason: "main horizontal split contains a non-Column child",
                }),
            })
            .collect::<Result<_, _>>()?;
        let total: f32 = widths.iter().copied().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "total dock column width overflowed",
            });
        }
        let ratios: Vec<f32> = widths.iter().map(|width| *width / total).collect();
        let area = self
            .main_area(side)
            .copied()
            .ok_or(CommandError::InvalidTarget {
                reason: "main dock area disappeared while synchronizing column ratios",
            })?;
        if let Some(Node::Split(split)) = self.nodes.get_mut(area.root) {
            if split.axis != Axis::X {
                return Err(CommandError::InvalidTarget {
                    reason: "main column split must use horizontal axis X",
                });
            }
            split.ratios = ratios;
        }
        Ok(())
    }
}
