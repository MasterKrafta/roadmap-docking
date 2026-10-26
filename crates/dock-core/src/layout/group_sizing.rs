use super::*;
use super::helpers::{normalized_ratios, ratio_issues};

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

    pub(super) fn apply_group_resize_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::ResizeGroups {
                column,
                boundary_index,
                delta,
                available_height,
            } => {
                self.apply_resize_groups_command(
                    column,
                    boundary_index,
                    delta,
                    available_height,
                    metrics,
                )
            }
            DockCommand::ResetColumnRatios {
                column,
                available_height,
            } => {
                self.apply_reset_column_ratios_command(column, available_height, metrics)
            }
            DockCommand::RestoreGroupRatios { column, ratios } => {
                self.apply_restore_group_ratios_command(column, ratios)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group resize dispatcher received an unsupported command",
            }),
        }
    }

    pub(super) fn apply_resize_groups_command(
        &mut self,
        column: &NodeId,
        boundary_index: &usize,
        delta: &f32,
        available_height: &f32,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !delta.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "delta",
                reason: "group boundary delta must be finite",
            });
        }
        if !available_height.is_finite() || *available_height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available group height must be finite and positive",
            });
        }
        let current = match self.nodes.get(*column) {
            Some(Node::Column(value)) => value.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "ResizeGroups target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *column }),
        };
        if *boundary_index >= current.groups.len().saturating_sub(1) {
            return Err(CommandError::InvalidTarget {
                reason: "group boundary index does not identify adjacent groups",
            });
        }
        let left_group = current.groups[*boundary_index];
        let right_group = current.groups[*boundary_index + 1];
        let heights =
            self.group_heights_for_available_height(*column, *available_height, metrics)?;
        let minimums = self.group_minimum_heights(*column, metrics)?;
        // group_minimum_heights already includes the effective fixed
        // height for collapsed groups; do not add that floor a second time.
        let required_height = minimums.iter().sum::<f32>();
        if !required_height.is_finite() || *available_height < required_height {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available height is below the sum of group minimum heights",
            });
        }
        if self.group_is_collapsed(left_group) || self.group_is_collapsed(right_group) {
            return Ok(CommandApplication {
                actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                inverse: None,
                persistent: true,
            });
        }

        let left_index = *boundary_index;
        let right_index = left_index + 1;
        let lower_delta = minimums[left_index] - heights[left_index];
        let upper_delta = heights[right_index] - minimums[right_index];
        if lower_delta > upper_delta {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available height cannot satisfy adjacent group minimums",
            });
        }
        let requested_delta = (*delta).clamp(lower_delta, upper_delta);
        if requested_delta == 0.0 {
            return Ok(CommandApplication {
                actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                inverse: None,
                persistent: true,
            });
        }
        if current.groups.iter().enumerate().any(|(index, group)| {
            !self.group_is_collapsed(*group)
                && heights[index] + PIXEL_TOLERANCE < minimums[index]
        }) {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "current group ratios place an expanded group below its minimum; reset ratios before resizing",
            });
        }

        // Ratios are weights among expanded groups. Transfer weight only
        // between this pair so every non-adjacent group's ratio remains
        // unchanged and the pair keeps its combined weight.
        let expanded_weight_sum: f32 = current
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| !self.group_is_collapsed(**group))
            .map(|(index, _)| current.group_ratios[index])
            .sum();
        let expanded_height: f32 = current
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| !self.group_is_collapsed(**group))
            .map(|(index, _)| heights[index])
            .sum();
        if expanded_weight_sum <= f32::EPSILON || expanded_height <= 0.0 {
            return Ok(CommandApplication {
                actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                inverse: None,
                persistent: true,
            });
        }

        let weight_delta = requested_delta * expanded_weight_sum / expanded_height;
        let mut new_ratios = current.group_ratios.clone();
        new_ratios[left_index] += weight_delta;
        new_ratios[right_index] -= weight_delta;
        if new_ratios[left_index] < 0.0 || new_ratios[right_index] < 0.0 {
            return Err(CommandError::InvalidValue {
                field: "delta",
                reason: "constrained resize produced a negative group weight",
            });
        }
        let final_heights = self.group_heights_for_ratios(
            *column,
            &new_ratios,
            *available_height,
            metrics,
        )?;
        let actual_delta = final_heights[left_index] - heights[left_index];
        if actual_delta == 0.0 || new_ratios == current.group_ratios {
            return Ok(CommandApplication {
                actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                inverse: None,
                persistent: true,
            });
        }
        if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
            value.group_ratios = new_ratios;
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::GroupBoundaryDelta(actual_delta)),
            inverse: Some(DockCommand::RestoreGroupRatios {
                column: *column,
                ratios: current.group_ratios,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_reset_column_ratios_command(
        &mut self,
        column: &NodeId,
        available_height: &f32,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !available_height.is_finite() || *available_height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available group height must be finite and positive",
            });
        }
        let current = match self.nodes.get(*column) {
            Some(Node::Column(value)) => value.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "ResetColumnRatios target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *column }),
        };
        // Validate the supplied height against fixed collapsed strips
        // and expanded minimums even when the reset itself becomes a no-op.
        let _current_heights =
            self.group_heights_for_available_height(*column, *available_height, metrics)?;
        if current
            .groups
            .iter()
            .all(|group| self.group_is_collapsed(*group))
        {
            return Ok(CommandApplication::persistent());
        }
        let heights = self.equal_group_heights(*column, *available_height, metrics)?;
        let ratios = self.group_ratios_from_heights(*column, &heights)?;
        if ratios == current.group_ratios {
            return Ok(CommandApplication::persistent());
        }
        if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
            value.group_ratios = ratios.clone();
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::GroupRatios(ratios)),
            inverse: Some(DockCommand::RestoreGroupRatios {
                column: *column,
                ratios: current.group_ratios,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_restore_group_ratios_command(
        &mut self,
        column: &NodeId,
        ratios: &Vec<f32>,
    ) -> Result<CommandApplication, CommandError> {
        let current = match self.nodes.get(*column) {
            Some(Node::Column(value)) => value.group_ratios.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "RestoreGroupRatios target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *column }),
        };
        let count = match self.nodes.get(*column) {
            Some(Node::Column(value)) => value.groups.len(),
            _ => 0,
        };
        if ratios.len() != count {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason: "ratio count must match group count",
            });
        }
        if let Some(reason) = ratio_issues(ratios, count).first() {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason,
            });
        }
        if current == *ratios {
            return Ok(CommandApplication::persistent());
        }
        if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
            value.group_ratios = ratios.clone();
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::GroupRatios(ratios.clone())),
            inverse: Some(DockCommand::RestoreGroupRatios {
                column: *column,
                ratios: current,
            }),
            persistent: true,
        })
    }
}


pub(super) fn distribute_heights_with_minimums(
    available_height: f32,
    minimums: &[f32],
    weights: &[f32],
) -> Result<Vec<f32>, CommandError> {
    if !available_height.is_finite() || available_height <= 0.0 {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "available group height must be finite and positive",
        });
    }
    if minimums.len() != weights.len() {
        return Err(CommandError::InvalidValue {
            field: "weights",
            reason: "group weights must match expanded group count",
        });
    }
    if minimums
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
        || weights
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(CommandError::InvalidValue {
            field: "weights",
            reason: "group minimums must be positive and weights finite/non-negative",
        });
    }
    if minimums.is_empty() {
        return Ok(Vec::new());
    }

    let minimum_total: f32 = minimums.iter().sum();
    if !minimum_total.is_finite() || minimum_total > available_height {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "available height is below the sum of expanded group minimums",
        });
    }

    let mut result = vec![0.0; minimums.len()];
    let mut active: Vec<usize> = (0..minimums.len()).collect();
    let mut remaining = available_height;
    while !active.is_empty() {
        let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
        let equal_share = weight_sum <= f32::EPSILON;
        let proposed: Vec<(usize, f32)> = active
            .iter()
            .map(|index| {
                let height = if equal_share {
                    remaining / active.len() as f32
                } else {
                    remaining * weights[*index] / weight_sum
                };
                (*index, height)
            })
            .collect();
        let mut constrained = Vec::new();
        for (index, height) in proposed {
            if height < minimums[index] {
                result[index] = minimums[index];
                remaining -= minimums[index];
                constrained.push(index);
            }
        }
        if constrained.is_empty() {
            let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
            let equal_share = weight_sum <= f32::EPSILON;
            for index in &active {
                result[*index] = if equal_share {
                    remaining / active.len() as f32
                } else {
                    remaining * weights[*index] / weight_sum
                };
            }
            break;
        }
        active.retain(|index| !constrained.contains(index));
    }

    if result
        .iter()
        .any(|height| !height.is_finite() || *height <= 0.0)
    {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "computed group heights are not finite and positive",
        });
    }
    Ok(result)
}

