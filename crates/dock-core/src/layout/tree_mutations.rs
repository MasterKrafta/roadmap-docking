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
