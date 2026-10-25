use super::*;
use super::helpers::{equal_ratios, normalized_ratios, side_index, split_sibling_ratios};

impl Layout {
    pub(super) fn group_node_for(&mut self, node: NodeId) -> Result<NodeId, CommandError> {
        match self.nodes.get(node).cloned() {
            Some(Node::Tabs(_)) => Ok(node),
            Some(Node::Panel(_)) => Ok(self.nodes.insert(Node::Tabs(crate::Tabs {
                children: vec![node],
                active_index: 0,
                collapsed: false,
            }))),
            Some(_) if self.profile == Profile::Generic => {
                Ok(self.nodes.insert(Node::Tabs(crate::Tabs {
                    children: vec![node],
                    active_index: 0,
                    collapsed: false,
                })))
            }
            Some(_) => Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "Photoshop columns and floating stacks can contain Tabs groups or Panel leaves only",
            }),
            None => Err(CommandError::MissingNode { node }),
        }
    }

    pub(super) fn insert_node_for_target(
        &mut self,
        node: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
        edge_ratio: Option<f32>,
    ) -> Result<(), CommandError> {
        self.validate_drop_target(target, metrics)?;
        match target {
            DropTarget::Tabs { group, index } => {
                if !matches!(self.nodes.get(node), Some(Node::Panel(_))) {
                    return Err(CommandError::InvalidTarget {
                        reason: "Tabs target accepts a Panel leaf; move a group to a group or column target",
                    });
                }
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::InvalidTarget {
                        reason: "tab target changed unexpectedly",
                    });
                };
                tabs.children.insert(index, node);
                tabs.active_index = index;
                Ok(())
            }
            DropTarget::GroupTabs { .. } => Err(CommandError::InvalidTarget {
                reason: "whole Tabs groups must be merged through MergeGroup",
            }),
            DropTarget::GroupEdge { group, zone } => {
                let group_node = self.group_node_for(node)?;
                self.insert_group_edge(group_node, group, zone, edge_ratio, metrics)
            }
            DropTarget::MainColumn { side, index } => {
                let group = self.group_node_for(node)?;
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![group],
                    group_ratios: vec![1.0],
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(side, index, column, None, metrics)
            }
            DropTarget::ColumnSlot { column, index } => {
                let group = self.group_node_for(node)?;
                self.insert_group_at_column(column, group, index, None)
            }
            DropTarget::RailSlot { column, index } => {
                if matches!(self.nodes.get(node), Some(Node::Panel(_))) {
                    self.insert_panel_for_rail_slot(node, column, index, metrics)?;
                    Ok(())
                } else {
                    let group_index = self.rail_slot_group_index(column, index)?;
                    let group = self.group_node_for(node)?;
                    self.insert_group_at_column(column, group, group_index, None)
                }
            }
            DropTarget::FreeArea => Err(CommandError::InvalidTarget {
                reason: "FreeArea must be resolved by Float with explicit window bounds",
            }),
        }
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


    pub(super) fn insert_existing_column_at(
        &mut self,
        side: DockSide,
        index: usize,
        new_column: NodeId,
        split_ratio: Option<(f32, usize)>,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if !matches!(self.nodes.get(new_column), Some(Node::Column(_))) {
            return Err(CommandError::InvalidTarget {
                reason: "inserted main column is not a Column node",
            });
        }
        let split_ratio = if let Some((ratio, target_index)) = split_ratio {
            if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                return Err(CommandError::InvalidValue {
                    field: "ratio",
                    reason: "split ratio must be between zero and one",
                });
            }
            Some((ratio, target_index))
        } else {
            None
        };

        let Some(area_id) = self.main_areas[side_index(side)] else {
            if index != 0 {
                return Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "first column on an empty dock must use index zero",
                });
            }
            let area_id = self.dock_areas.insert(DockArea::main(side, new_column));
            self.main_areas[side_index(side)] = Some(area_id);
            return Ok(());
        };
        let area_root = self
            .dock_areas
            .get(area_id)
            .ok_or(CommandError::InvalidTarget {
                reason: "main dock side references a missing area",
            })?
            .root;

        match self.nodes.get(area_root).cloned() {
            Some(Node::Column(_)) => {
                if index > 1 {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "column insertion index is outside the dock",
                    });
                }
                if let Some((ratio, target_index)) = split_ratio {
                    if target_index != 0 {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target does not identify the existing column",
                        });
                    }
                    self.resize_inserted_column_for_split_ratio(
                        area_root,
                        new_column,
                        ratio,
                        metrics,
                    )?;
                }
                let children = if index == 0 {
                    vec![new_column, area_root]
                } else {
                    vec![area_root, new_column]
                };
                let split = self.nodes.insert(Node::Split(crate::Split {
                    axis: Axis::X,
                    ratios: equal_ratios(2),
                    children,
                }));
                if let Some(area) = self.dock_areas.get_mut(area_id) {
                    area.root = split;
                }
                self.sync_main_column_ratios(new_column)?;
                Ok(())
            }
            Some(Node::Split(mut split)) if split.axis == Axis::X => {
                if index > split.children.len() {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "column insertion index is outside the dock",
                    });
                }
                if split
                    .children
                    .iter()
                    .any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split contains a non-Column child",
                    });
                }
                let old_len = split.children.len();
                if let Some((ratio, target_index)) = split_ratio {
                    if target_index >= old_len {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target or column insertion index is outside the dock",
                        });
                    }
                    let target_column = split.children[target_index];
                    self.resize_inserted_column_for_split_ratio(
                        target_column,
                        new_column,
                        ratio,
                        metrics,
                    )?;
                }
                split.children.insert(index, new_column);
                // Main-column Split(X) ratios mirror visible column widths. They
                // are recalculated after the tree is updated below.
                split.ratios = equal_ratios(old_len + 1);
                if let Some(Node::Split(current)) = self.nodes.get_mut(area_root) {
                    *current = split;
                } else {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split changed unexpectedly",
                    });
                }
                self.sync_main_column_ratios(new_column)?;
                Ok(())
            }
            Some(_) => Err(CommandError::InvalidTarget {
                reason: "main dock root must be a Column or Split(X) of Columns",
            }),
            None => Err(CommandError::InvalidTarget {
                reason: "main dock root is missing",
            }),
        }
    }

    pub(super) fn insert_group_at_column(
        &mut self,
        column_id: NodeId,
        group_id: NodeId,
        index: usize,
        split_ratio: Option<(f32, usize)>,
    ) -> Result<(), CommandError> {
        if group_id == column_id {
            return Err(CommandError::InvalidTarget {
                reason: "a column cannot contain itself",
            });
        }
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "group insertion target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if index > column.groups.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "group insertion index is outside the column",
            });
        }
        if column.groups.contains(&group_id) {
            return Err(CommandError::InvalidTarget {
                reason: "group is already in the target column",
            });
        }
        let mut groups = column.groups.clone();
        groups.insert(index, group_id);
        let ratios = if let Some((ratio, target_index)) = split_ratio {
            if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                return Err(CommandError::InvalidValue {
                    field: "ratio",
                    reason: "split ratio must be between zero and one",
                });
            }
            let old_len = column.groups.len();
            if target_index >= old_len {
                return Err(CommandError::InvalidTarget {
                    reason: "split ratio target does not identify an existing group",
                });
            }
            split_sibling_ratios(
                &column.group_ratios,
                old_len,
                target_index,
                index,
                ratio,
            )
        } else if column.groups.is_empty() {
            vec![1.0]
        } else {
            let old_ratios = normalized_ratios(
                column.groups.iter().enumerate().map(|(index, _)| column.group_ratios.get(index).copied()).collect(),
            );
            let old_scale = column.groups.len() as f32 / groups.len() as f32;
            let mut preserved = Vec::with_capacity(groups.len());
            for group_index in 0..groups.len() {
                if group_index == index {
                    preserved.push(1.0 / groups.len() as f32);
                } else {
                    let old_index = if group_index < index { group_index } else { group_index - 1 };
                    preserved.push(old_ratios[old_index] * old_scale);
                }
            }
            normalized_ratios(preserved.into_iter().map(Some).collect())
        };
        let Some(Node::Column(column)) = self.nodes.get_mut(column_id) else {
            return Err(CommandError::InvalidTarget {
                reason: "target column changed unexpectedly",
            });
        };
        column.groups = groups;
        column.group_ratios = ratios;
        Ok(())
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
}
