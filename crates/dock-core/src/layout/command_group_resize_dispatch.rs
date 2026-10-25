use super::*;
use super::helpers::ratio_issues;

impl Layout {
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
}
