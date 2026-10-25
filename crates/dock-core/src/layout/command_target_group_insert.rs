use super::*;
use super::helpers::{normalized_ratios, split_sibling_ratios};

impl Layout {
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
}
