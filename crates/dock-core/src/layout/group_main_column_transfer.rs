use super::*;
use crate::AllowedArea;

pub(super) struct GroupMainColumnSource {
    pub(super) column: Option<NodeId>,
    pub(super) position: Option<(DockSide, usize)>,
    pub(super) group_index: Option<usize>,
    pub(super) is_sole_group: bool,
}

impl Layout {
    pub(super) fn apply_move_group_to_main_column_command(
        &mut self,
        group: &NodeId,
        side: &DockSide,
        index: &usize,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
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

    pub(super) fn move_group_to_new_main_column(
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

    pub(super) fn group_main_column_source(&self, group: NodeId) -> GroupMainColumnSource {
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

    pub(super) fn inverse_for_move_group_to_main_column(
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

    pub(super) fn try_reorder_sole_group_main_column(
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
