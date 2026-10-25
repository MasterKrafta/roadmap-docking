use super::*;

pub(super) struct GroupMainColumnSource {
    pub(super) column: Option<NodeId>,
    pub(super) position: Option<(DockSide, usize)>,
    pub(super) group_index: Option<usize>,
    pub(super) is_sole_group: bool,
}

impl Layout {
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
}
