use super::*;

impl Layout {
    pub(super) fn group_is_collapsed(&self, group_id: NodeId) -> bool {
        matches!(self.nodes.get(group_id), Some(Node::Tabs(tabs)) if tabs.collapsed)
    }

    pub(super) fn group_minimum_heights(
        &self,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Vec<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "group height constraints require a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        column
            .groups
            .iter()
            .map(|group| {
                // Use the same minimum-size contract for expanded and collapsed
                // groups. Resize commands account for every group's floor once.
                self.minimum_node_size(*group, metrics)
                    .map(|size| size.height)
            })
            .collect()
    }

    pub(super) fn group_heights_for_available_height(
        &self,
        column_id: NodeId,
        available_height: f32,
        metrics: &DockMetrics,
    ) -> Result<Vec<f32>, CommandError> {
        let ratios = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "group height calculation requires a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        self.group_heights_for_ratios(column_id, &ratios, available_height, metrics)
    }

    pub(super) fn group_heights_for_ratios(
        &self,
        column_id: NodeId,
        ratios: &[f32],
        available_height: f32,
        metrics: &DockMetrics,
    ) -> Result<Vec<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "group height calculation requires a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if ratios.len() != column.groups.len() {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason: "ratio count must match group count",
            });
        }
        let mut heights = vec![0.0; column.groups.len()];
        let mut expanded_indices = Vec::new();
        let mut weights = Vec::new();
        let mut fixed_height = 0.0_f32;
        for (index, group) in column.groups.iter().copied().enumerate() {
            if self.group_is_collapsed(group) {
                let minimum_height = self.minimum_node_size(group, metrics)?.height;
                heights[index] = minimum_height;
                fixed_height += minimum_height;
            } else {
                expanded_indices.push(index);
                weights.push(ratios[index]);
            }
        }
        let free_height = available_height - fixed_height;
        if !free_height.is_finite() || free_height < 0.0 {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available height cannot fit the fixed collapsed groups",
            });
        }
        if expanded_indices.is_empty() {
            return Ok(heights);
        }
        if free_height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available height leaves no space for expanded groups",
            });
        }
        let total_weight: f32 = weights.iter().copied().sum();
        if !total_weight.is_finite() || total_weight < 0.0 {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason: "expanded group weights must have a finite sum",
            });
        }
        for (index, weight) in expanded_indices.into_iter().zip(weights) {
            heights[index] = if total_weight <= f32::EPSILON {
                free_height
                    / column
                        .groups
                        .iter()
                        .filter(|group| !self.group_is_collapsed(**group))
                        .count() as f32
            } else {
                free_height * weight / total_weight
            };
        }
        Ok(heights)
    }
}
