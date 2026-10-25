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
                if source_parent == Some(*column)
                    && let Some(application) = self.try_reorder_group_within_column(
                        *group,
                        *column,
                        *index,
                        &target_groups,
                    )?
                {
                    return Ok(application);
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
                self.move_group_to_existing_column(*group, *column, *index)?;
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

                self.move_group_to_new_main_column(*group, *side, *index, metrics)?;
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

    fn move_group_to_existing_column(
        &mut self,
        group: NodeId,
        column: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        self.remember_subtree_dock_locations(group);
        self.detach_node(group)?;
        let new_len = match self.nodes.get(column) {
            Some(Node::Column(value)) => value.groups.len(),
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "MoveGroup target changed unexpectedly",
                });
            }
        };
        self.insert_group_at_column(column, group, index.min(new_len), None)?;
        self.remember_subtree_dock_locations(group);
        Ok(())
    }

    fn move_group_to_new_main_column(
        &mut self,
        group: NodeId,
        side: DockSide,
        index: usize,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        self.remember_subtree_dock_locations(group);
        self.detach_node(group)?;
        let new_column = self.nodes.insert(Node::Column(crate::Column {
            width: metrics.default_column_width,
            mode: ColumnMode::Expanded,
            groups: vec![group],
            group_ratios: vec![1.0],
            last_expanded_width: Some(metrics.default_column_width),
        }));
        self.insert_existing_column_at(side, index, new_column, None, metrics)?;
        self.remember_subtree_dock_locations(group);
        Ok(())
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

}
