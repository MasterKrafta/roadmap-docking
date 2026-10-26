use super::*;
use super::helpers::normalized_ratios;

impl Layout {
        pub(super) fn equal_group_heights(
            &self,
            column_id: NodeId,
            available_height: f32,
            metrics: &DockMetrics,
        ) -> Result<Vec<f32>, CommandError> {
            let column = match self.nodes.get(column_id) {
                Some(Node::Column(column)) => column,
                Some(_) => {
                    return Err(CommandError::InvalidTarget {
                        reason: "group height reset requires a Column node",
                    });
                }
                None => return Err(CommandError::MissingNode { node: column_id }),
            };
            let mut heights = vec![0.0; column.groups.len()];
            let mut expanded_indices = Vec::new();
            let mut fixed_height = 0.0_f32;
            for (index, group) in column.groups.iter().copied().enumerate() {
                if self.group_is_collapsed(group) {
                    let minimum_height = self.minimum_node_size(group, metrics)?.height;
                    heights[index] = minimum_height;
                    fixed_height += minimum_height;
                } else {
                    expanded_indices.push(index);
                }
            }
            if expanded_indices.is_empty() {
                return Ok(heights);
            }
            let free_height = available_height - fixed_height;
            let all_minimums = self.group_minimum_heights(column_id, metrics)?;
            let minimums: Vec<f32> = expanded_indices
                .iter()
                .map(|index| all_minimums[*index])
                .collect();
            let equal_weights = vec![1.0; expanded_indices.len()];
            let expanded_heights =
                distribute_heights_with_minimums(free_height, &minimums, &equal_weights)?;
            for (index, height) in expanded_indices.into_iter().zip(expanded_heights) {
                heights[index] = height;
            }
            Ok(heights)
        }
    
        pub(super) fn group_ratios_from_heights(
            &self,
            column_id: NodeId,
            heights: &[f32],
        ) -> Result<Vec<f32>, CommandError> {
            let column = match self.nodes.get(column_id) {
                Some(Node::Column(column)) => column,
                Some(_) => {
                    return Err(CommandError::InvalidTarget {
                        reason: "group ratio calculation requires a Column node",
                    });
                }
                None => return Err(CommandError::MissingNode { node: column_id }),
            };
            if heights.len() != column.groups.len() {
                return Err(CommandError::InvalidValue {
                    field: "heights",
                    reason: "group height count must match group count",
                });
            }
            let expanded_height: f32 = column
                .groups
                .iter()
                .enumerate()
                .filter(|(_, group)| !self.group_is_collapsed(**group))
                .map(|(index, _)| heights[index])
                .sum();
            if expanded_height <= 0.0 {
                return Ok(column.group_ratios.clone());
            }
    
            // Reset the visible groups without discarding the saved weight of a
            // collapsed group. Once expanded again, it should recover its share
            // instead of being constrained to its minimum height.
            let collapsed_weight: f32 = column
                .groups
                .iter()
                .enumerate()
                .filter(|(_, group)| self.group_is_collapsed(**group))
                .map(|(index, _)| column.group_ratios[index])
                .sum();
            let remaining_weight = (1.0 - collapsed_weight).max(0.0);
            let mut ratios = vec![0.0; column.groups.len()];
            for (index, group) in column.groups.iter().copied().enumerate() {
                if self.group_is_collapsed(group) {
                    ratios[index] = column.group_ratios[index];
                } else {
                    ratios[index] = heights[index] / expanded_height * remaining_weight;
                }
            }
            Ok(normalized_ratios(ratios.into_iter().map(Some).collect()))
        }
}
