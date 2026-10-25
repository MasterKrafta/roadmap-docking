use super::*;

impl Layout {
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
