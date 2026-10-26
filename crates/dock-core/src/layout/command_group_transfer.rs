use super::*;



impl Layout {
    pub(super) fn apply_group_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MoveGroup { group, column, index } => {
                self.apply_move_group_command(group, column, index)
            }
            DockCommand::MoveGroupToMainColumn { group, side, index } => {
                self.apply_move_group_to_main_column_command(group, side, index, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group transfer dispatcher received an unsupported command",
            }),
        }
    }

    pub(super) fn apply_move_group_command(
        &mut self,
        group: &NodeId,
        column: &NodeId,
        index: &usize,
    ) -> Result<CommandApplication, CommandError> {
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

    pub(super) fn move_group_to_existing_column(
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

    pub(super) fn try_reorder_group_within_column(
        &mut self,
        group: NodeId,
        column: NodeId,
        index: usize,
        target_groups: &[NodeId],
    ) -> Result<Option<CommandApplication>, CommandError> {
        let Some(source_index) = target_groups.iter().position(|id| *id == group) else {
            return Ok(None);
        };
        if index == source_index || index == source_index + 1 {
            return Ok(Some(CommandApplication::persistent()));
        }

        let adjusted_index = if index > source_index {
            index - 1
        } else {
            index
        };
        let inverse_index = if source_index > adjusted_index {
            source_index + 1
        } else {
            source_index
        };
        let inverse = Some(DockCommand::MoveGroup {
            group,
            column,
            index: inverse_index,
        });
        let Some(Node::Column(target_column)) = self.nodes.get_mut(column) else {
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
        self.remember_subtree_dock_locations(group);

        Ok(Some(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        }))
    }

}