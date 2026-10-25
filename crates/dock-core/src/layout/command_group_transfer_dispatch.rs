use super::*;
use crate::AllowedArea;

struct GroupMainColumnSource {
    column: Option<NodeId>,
    position: Option<(DockSide, usize)>,
    group_index: Option<usize>,
    is_sole_group: bool,
}

impl Layout {
    pub(super) fn apply_group_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MoveGroup {
                group,
                column,
                index,
            } => {
                match self.nodes.get(*group) {
                    Some(Node::Tabs(_)) => {}
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "MoveGroup source must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                let target_groups = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.groups.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "MoveGroup target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if *index > target_groups.len() {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "group insertion index is outside the column",
                    });
                }
                let target_area = self.allowed_area_for_column_target(*column)?;
                let moving_panels = self.panel_ids_in_subtree(*group);
                self.validate_panel_ids_allowed_for_area(&moving_panels, target_area)?;
                let source_parent = self.find_parent(*group);
                if source_parent == Some(*column) {
                    let source_index = target_groups.iter().position(|id| id == group);
                    if let Some(source_index) = source_index {
                        if *index == source_index || *index == source_index + 1 {
                            return Ok(CommandApplication::persistent());
                        }
                        let adjusted_index = if *index > source_index {
                            *index - 1
                        } else {
                            *index
                        };
                        let inverse_index = if source_index > adjusted_index {
                            source_index + 1
                        } else {
                            source_index
                        };
                        let inverse = Some(DockCommand::MoveGroup {
                            group: *group,
                            column: *column,
                            index: inverse_index,
                        });
                        let Some(Node::Column(target_column)) = self.nodes.get_mut(*column) else {
                            return Err(CommandError::InvalidTarget {
                                reason: "MoveGroup target changed unexpectedly",
                            });
                        };
                        let moved_group = target_column.groups.remove(source_index);
                        let moved_ratio = target_column.group_ratios.remove(source_index);
                        target_column.groups.insert(adjusted_index, moved_group);
                        target_column
                            .group_ratios
                            .insert(adjusted_index, moved_ratio);
                        self.remember_subtree_dock_locations(*group);
                        return Ok(CommandApplication {
                            actual_value: None,
                            inverse,
                            persistent: true,
                        });
                    }
                }
                let source_column = source_parent.and_then(|parent| match self.nodes.get(parent) {
                    Some(Node::Column(column)) if column.groups.len() > 1 => column
                        .groups
                        .iter()
                        .position(|id| *id == *group)
                        .map(|index| (parent, index)),
                    _ => None,
                });
                let inverse =
                    source_column.map(|(source_column, source_index)| DockCommand::MoveGroup {
                        group: *group,
                        column: source_column,
                        index: source_index,
                    });
                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                let new_len = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.groups.len(),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "MoveGroup target changed unexpectedly",
                        });
                    }
                };
                self.insert_group_at_column(*column, *group, (*index).min(new_len), None)?;
                self.remember_subtree_dock_locations(*group);
                Ok(CommandApplication {
                    actual_value: None,
                    inverse,
                    persistent: true,
                })
            }
            DockCommand::MoveGroupToMainColumn { group, side, index } => {
                if !matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
                    return Err(if self.nodes.contains_key(*group) {
                        CommandError::InvalidTarget {
                            reason: "MoveGroupToMainColumn source must be a Tabs node",
                        }
                    } else {
                        CommandError::MissingNode { node: *group }
                    });
                }

                let target_columns = self.main_columns(*side)?;
                if *index > target_columns.len() {
                    return Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "main-column insertion index is outside the dock",
                    });
                }

                let moving_panels = self.panel_ids_in_subtree(*group);
                self.validate_panel_ids_allowed_for_area(&moving_panels, AllowedArea::MAIN_DOCK)?;

                let source = self.group_main_column_source(*group);

                if let Some(application) = self.try_reorder_sole_group_main_column(
                    *group,
                    *side,
                    *index,
                    source.column,
                    source.position,
                    source.is_sole_group,
                )? {
                    return Ok(application);
                }

                let inverse = self.inverse_for_move_group_to_main_column(*group, &source);

                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                let new_column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![*group],
                    group_ratios: vec![1.0],
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(*side, *index, new_column, None, metrics)?;
                self.remember_subtree_dock_locations(*group);
                Ok(CommandApplication {
                    actual_value: None,
                    inverse,
                    persistent: true,
                })
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group transfer dispatcher received an unsupported command",
            }),
        }
    }

    fn group_main_column_source(&self, group: NodeId) -> GroupMainColumnSource {
        let column = self.find_owner_column(group);
        let position = column.and_then(|column| self.main_column_position(column));
        let group_index = column.and_then(|column| match self.nodes.get(column) {
            Some(Node::Column(value)) => value.groups.iter().position(|id| *id == group),
            _ => None,
        });
        let is_sole_group = column.is_some_and(|column| {
            matches!(self.nodes.get(column), Some(Node::Column(value))
                if value.groups.len() == 1 && value.groups[0] == group)
        });

        GroupMainColumnSource {
            column,
            position,
            group_index,
            is_sole_group,
        }
    }

    fn inverse_for_move_group_to_main_column(
        &self,
        group: NodeId,
        source: &GroupMainColumnSource,
    ) -> Option<DockCommand> {
        match (source.column, source.group_index, source.position) {
            (Some(column), Some(group_index), _)
                if matches!(
                    self.nodes.get(column),
                    Some(Node::Column(value)) if value.groups.len() > 1
                ) =>
            {
                Some(DockCommand::MoveGroup {
                    group,
                    column,
                    index: group_index,
                })
            }
            (_, _, Some((source_side, source_index))) => {
                Some(DockCommand::MoveGroupToMainColumn {
                    group,
                    side: source_side,
                    index: source_index,
                })
            }
            _ => None,
        }
    }

    fn try_reorder_sole_group_main_column(
        &mut self,
        group: NodeId,
        side: DockSide,
        index: usize,
        source_column: Option<NodeId>,
        source_position: Option<(DockSide, usize)>,
        source_is_sole_group: bool,
    ) -> Result<Option<CommandApplication>, CommandError> {
        let Some((source_side, source_index)) = source_position else {
            return Ok(None);
        };
        if !source_is_sole_group || source_side != side {
            return Ok(None);
        }

        // An insertion slot immediately before or after the column is its current place.
        if index == source_index || index == source_index + 1 {
            return Ok(Some(CommandApplication::persistent()));
        }

        let area_root = self.main_area(side).map(|area| area.root).ok_or(
            CommandError::InvalidTarget {
                reason: "destination main dock area disappeared",
            },
        )?;
        let Some(Node::Split(mut split)) = self.nodes.get(area_root).cloned() else {
            // A single-column dock has only its current slot; a valid out-of-range
            // insertion was rejected before attempting this reorder.
            return Ok(Some(CommandApplication::persistent()));
        };
        if split.axis != Axis::X
            || source_column.is_none_or(|column| !split.children.contains(&column))
        {
            return Err(CommandError::InvalidTarget {
                reason: "main dock column order must be represented by Split(X)",
            });
        }
        let Some(ratio_index) = split
            .children
            .iter()
            .position(|id| Some(*id) == source_column)
        else {
            return Err(CommandError::InvalidTarget {
                reason: "source column is absent from the main dock split",
            });
        };
        if ratio_index >= split.ratios.len() {
            return Err(CommandError::InvalidTarget {
                reason: "main dock column ratio is missing",
            });
        }

        let moved_column = split.children.remove(ratio_index);
        let moved_ratio = split.ratios.remove(ratio_index);
        let adjusted_index = if index > source_index {
            index - 1
        } else {
            index
        };
        if adjusted_index > split.children.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "adjusted main-column insertion index is outside the dock",
            });
        }
        split.children.insert(adjusted_index, moved_column);
        split.ratios.insert(adjusted_index, moved_ratio);
        if let Some(Node::Split(current)) = self.nodes.get_mut(area_root) {
            *current = split;
        }
        self.remember_subtree_dock_locations(group);
        let inverse_index = if source_index > adjusted_index {
            source_index + 1
        } else {
            source_index
        };

        Ok(Some(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::MoveGroupToMainColumn {
                group,
                side,
                index: inverse_index,
            }),
            persistent: true,
        }))
    }

}
