//! Group sizing, resizing, minimum-size calculations, and ratio management.

use std::collections::HashMap;

use super::helpers::{normalized_ratios, ratio_issues};
use super::*;

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
            } => self.apply_resize_groups_command(
                column,
                boundary_index,
                delta,
                available_height,
                metrics,
            ),
            DockCommand::ResetColumnRatios {
                column,
                available_height,
            } => self.apply_reset_column_ratios_command(column, available_height, metrics),
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
            !self.group_is_collapsed(*group) && heights[index] + PIXEL_TOLERANCE < minimums[index]
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
        let final_heights =
            self.group_heights_for_ratios(*column, &new_ratios, *available_height, metrics)?;
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
                reason: reason.as_str(),
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

impl Layout {
    /// Computes the structural minimum layout size using the same rules as geometry.
    /// Standalone Tabs reserve separate header and tab-strip rows; a Tabs node owned
    /// directly by a Column uses one combined row, matching Column Group geometry.
    /// Expanded Columns reserve their own header and one compact strip per Group;
    /// rail Columns reserve only their flattened panel slots.
    pub(crate) fn minimum_node_size(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
        self.minimum_node_size_with_cache(node_id, metrics, &mut HashMap::new())
    }

    /// Memoized form for immutable operations such as a single geometry pass.
    /// The caller-provided cache must be discarded whenever the layout or metrics
    /// change. Only successfully computed sizes are cached.
    pub(crate) fn minimum_node_size_with_cache(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
        cache: &mut HashMap<NodeId, Size>,
    ) -> Result<Size, CommandError> {
        if let Some(size) = cache.get(&node_id).copied() {
            return Ok(size);
        }
        #[cfg(test)]
        crate::compute_rects::record_recursive_minimum_node_size(node_id);
        let node = self
            .nodes
            .get(node_id)
            .ok_or(CommandError::MissingNode { node: node_id })?;
        let size = match node {
            Node::Panel(panel) => {
                self.panel_instances
                    .get(panel.panel_id)
                    .ok_or(CommandError::MissingPanel {
                        panel: panel.panel_id,
                    })?
                    .constraints
                    .min_size
            }
            Node::Tabs(tabs) if tabs.collapsed => {
                Size::new(0.0, metrics.effective_collapsed_group_height())
            }
            Node::Tabs(tabs) => {
                let mut width = 0.0_f32;
                let mut content_height = 0.0_f32;
                for child in &tabs.children {
                    let child_size = self.minimum_node_size_with_cache(*child, metrics, cache)?;
                    width = width.max(child_size.width);
                    content_height = content_height.max(child_size.height);
                }
                let chrome_height = if self.find_owner_column(node_id).is_some() {
                    metrics.header_height.max(metrics.tab_strip_height)
                } else {
                    metrics.header_height + metrics.tab_strip_height
                };
                Size::new(
                    width,
                    metrics.min_group_height.max(chrome_height + content_height),
                )
            }
            Node::Split(split) => {
                let mut width = 0.0_f32;
                let mut height = 0.0_f32;
                for child in &split.children {
                    let child_size = self.minimum_node_size_with_cache(*child, metrics, cache)?;
                    match split.axis {
                        Axis::X => {
                            width += child_size.width;
                            height = height.max(child_size.height);
                        }
                        Axis::Y => {
                            width = width.max(child_size.width);
                            height += child_size.height;
                        }
                    }
                }
                let gaps =
                    metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
                match split.axis {
                    Axis::X => width += gaps,
                    Axis::Y => height += gaps,
                }
                Size::new(width, height)
            }
            Node::Column(column) if column.mode.is_rail() => {
                let width = if column.mode == ColumnMode::IconsOnly {
                    metrics.rail_width
                } else {
                    metrics.min_column_width
                };
                let slot_count = self.column_display_order(node_id)?.len();
                Size::new(
                    width,
                    metrics.effective_collapsed_group_height() * slot_count as f32,
                )
            }
            Node::Column(column) => {
                // An expanded Column can always fall back to one compact strip per
                // Group. Its structural minimum therefore reserves the Column header
                // and those strips, not the full minimum height of every Panel view.
                let height = if column.groups.is_empty() {
                    0.0
                } else {
                    metrics.column_header_height
                        + metrics.effective_collapsed_group_height()
                            * column.groups.len() as f32
                };
                Size::new(column.width, height)
            }
        };

        if !size.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed minimum size overflowed",
            });
        }
        if size.width < 0.0 || size.height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed minimum size has invalid dimensions",
            });
        }
        cache.insert(node_id, size);
        Ok(size)
    }

    /// Computes a node's minimum geometry for a Floating composition.
    ///
    /// Unlike Main geometry, expanded Tabs reserve only the active panel's
    /// content height. Their width still accommodates every tab, and all Dock
    /// chrome remains part of the calculated group size.
    pub(crate) fn minimum_floating_node_size_with_cache(
        &self,
        node_id: NodeId,
        floating_root: NodeId,
        metrics: &DockMetrics,
        cache: &mut HashMap<NodeId, Size>,
    ) -> Result<Size, CommandError> {
        if let Some(size) = cache.get(&node_id).copied() {
            return Ok(size);
        }

        let node = self
            .nodes
            .get(node_id)
            .ok_or(CommandError::MissingNode { node: node_id })?;
        let size = match node {
            Node::Panel(panel) => self
                .panel_instances
                .get(panel.panel_id)
                .ok_or(CommandError::MissingPanel {
                    panel: panel.panel_id,
                })?
                .constraints
                .min_size,
            Node::Tabs(tabs) if tabs.collapsed => {
                Size::new(0.0, metrics.effective_collapsed_group_height())
            }
            Node::Tabs(tabs) => {
                let mut width = 0.0_f32;
                for child in &tabs.children {
                    width = width.max(
                        self.minimum_node_size_with_cache(*child, metrics, cache)?.width,
                    );
                }
                let active_child = tabs.active_child().ok_or(CommandError::InvalidTarget {
                    reason: "Floating Tabs group has no active panel",
                })?;
                let active_minimum = self.minimum_floating_node_size_with_cache(
                    active_child,
                    floating_root,
                    metrics,
                    cache,
                )?;
                let is_column_group =
                    node_id == floating_root || self.find_owner_column(node_id).is_some();
                let chrome_height = if is_column_group {
                    metrics.header_height.max(metrics.tab_strip_height)
                } else {
                    metrics.header_height + metrics.tab_strip_height
                };
                Size::new(
                    width.max(active_minimum.width),
                    metrics
                        .min_group_height
                        .max(chrome_height + active_minimum.height),
                )
            }
            Node::Split(split) => {
                let mut width = 0.0_f32;
                let mut height = 0.0_f32;
                for child in &split.children {
                    let child_size = self.minimum_floating_node_size_with_cache(
                        *child,
                        floating_root,
                        metrics,
                        cache,
                    )?;
                    match split.axis {
                        Axis::X => {
                            width += child_size.width;
                            height = height.max(child_size.height);
                        }
                        Axis::Y => {
                            width = width.max(child_size.width);
                            height += child_size.height;
                        }
                    }
                }
                let gaps =
                    metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
                match split.axis {
                    Axis::X => width += gaps,
                    Axis::Y => height += gaps,
                }
                Size::new(width, height)
            }
            Node::Column(column) if column.mode.is_rail() => {
                self.minimum_node_size_with_cache(node_id, metrics, cache)?
            }
            Node::Column(column) => {
                let mut content_minimum = Size::ZERO;
                let mut group_minimums = Vec::with_capacity(column.groups.len());
                for group in &column.groups {
                    let group_minimum = match self.nodes.get(*group) {
                        Some(Node::Tabs(tabs)) if tabs.collapsed => {
                            let mut width = 0.0_f32;
                            for child in &tabs.children {
                                width = width.max(
                                    self.minimum_node_size_with_cache(*child, metrics, cache)?.width,
                                );
                            }
                            Size::new(width, metrics.effective_collapsed_group_height())
                        }
                        Some(_) => self.minimum_floating_node_size_with_cache(
                            *group,
                            floating_root,
                            metrics,
                            cache,
                        )?,
                        None => return Err(CommandError::MissingNode { node: *group }),
                    };
                    group_minimums.push(group_minimum);
                    content_minimum.width = content_minimum.width.max(group_minimum.width);
                }

                let height = match column.groups.as_slice() {
                    // A Column always reserves its CompositionHeader, even when
                    // its only Group is collapsed to a compact tab strip.
                    [_] => metrics.column_header_height + group_minimums[0].height,
                    groups => metrics.column_header_height
                        + metrics.effective_collapsed_group_height() * groups.len() as f32,
                };
                Size::new(content_minimum.width, height)
            }
        };

        if !size.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed Floating minimum size overflowed",
            });
        }
        if size.width < 0.0 || size.height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed Floating minimum size has invalid dimensions",
            });
        }
        cache.insert(node_id, size);
        Ok(size)
    }

    /// Computes the minimum client size of one Column in a Floating composition.
    ///
    /// The width belongs to this Column, while the height is calculated with
    /// the same Floating-root rules used by the containing composition.
    pub fn floating_column_minimum_size(
        &self,
        group_id: crate::FloatingGroupId,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
        let floating = self
            .floating_groups
            .get(group_id)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group: group_id })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .map(|area| area.root)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?;
        let is_root_column = root == column_id
            && matches!(self.nodes.get(root), Some(Node::Column(_)));
        let is_split_column = matches!(
            self.nodes.get(root),
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split.children.iter().any(|child| *child == column_id)
                    && matches!(self.nodes.get(column_id), Some(Node::Column(_)))
        );
        if !is_root_column && !is_split_column {
            return Err(CommandError::InvalidTarget {
                reason: "target Column does not belong to the Floating composition root",
            });
        }

        self.minimum_floating_node_size_with_cache(
            column_id,
            root,
            metrics,
            &mut HashMap::new(),
        )
    }

    /// Computes a floating composition's minimum client size.
    ///
    /// Floating windows have no generic title row. A root Column includes its
    /// ColumnHeader recursively; a root Tabs group receives the shared
    /// CompositionHeader row after its group minimum is calculated.
    pub(crate) fn minimum_floating_size(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
        self.minimum_floating_size_with_cache(node_id, metrics, &mut HashMap::new())
    }

    /// Computes the natural height of one Floating composition after a tab
    /// activation, including Dock chrome but excluding the native window frame.
    ///
    /// The requested tab is used as the active tab for this calculation without
    /// mutating Layout. Other FloatingGroups (including chained windows) are
    /// deliberately not part of this subtree calculation.
    pub fn floating_preferred_height_for_tab(
        &self,
        group_id: crate::FloatingGroupId,
        tabs_group: NodeId,
        panel: crate::PanelId,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let floating = self
            .floating_groups
            .get(group_id)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group: group_id })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .map(|area| area.root)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?;

        if !self.floating_node_contains_node(root, tabs_group) {
            return Err(CommandError::InvalidTarget {
                reason: "tab group does not belong to the requested Floating composition",
            });
        }
        let tabs = match self.nodes.get(tabs_group) {
            Some(Node::Tabs(tabs)) => tabs,
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "preferred Floating height requires a Tabs group",
                });
            }
        };
        let panel_node = tabs
            .children
            .iter()
            .find(|child| {
                matches!(
                    self.nodes.get(**child),
                    Some(Node::Panel(value)) if value.panel_id == panel
                )
            })
            .copied()
            .ok_or(CommandError::InvalidTarget {
                reason: "panel is not a tab in the requested Floating group",
            })?;

        self.preferred_floating_height_for_root(
            root,
            Some((tabs_group, panel_node)),
            metrics,
        )
    }

    /// Computes the natural height for a subtree about to become a Floating
    /// composition. The caller supplies the node that will be the final root,
    /// after profile-specific wrappers have been applied.
    pub fn floating_preferred_height_for_node(
        &self,
        root: NodeId,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        if !self.nodes.contains_key(root) {
            return Err(CommandError::MissingNode { node: root });
        }
        self.preferred_floating_height_for_root(root, None, metrics)
    }

    fn preferred_floating_height_for_root(
        &self,
        root: NodeId,
        active_override: Option<(NodeId, NodeId)>,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let content_height = self.preferred_floating_node_height(
            root,
            root,
            active_override,
            metrics,
        )?;
        let composition_header = if matches!(self.nodes.get(root), Some(Node::Tabs(_))) {
            metrics.column_header_height
        } else {
            0.0
        };
        let height = content_height + composition_header;
        if !height.is_finite() || height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed preferred Floating height is invalid",
            });
        }
        Ok(height)
    }

    fn floating_node_contains_node(&self, root: NodeId, target: NodeId) -> bool {
        if root == target {
            return true;
        }
        match self.nodes.get(root) {
            Some(Node::Tabs(tabs)) => tabs
                .children
                .iter()
                .any(|child| self.floating_node_contains_node(*child, target)),
            Some(Node::Split(split)) => split
                .children
                .iter()
                .any(|child| self.floating_node_contains_node(*child, target)),
            Some(Node::Column(column)) => column
                .groups
                .iter()
                .any(|child| self.floating_node_contains_node(*child, target)),
            Some(Node::Panel(_)) | None => false,
        }
    }

    fn preferred_floating_node_height(
        &self,
        node_id: NodeId,
        floating_root: NodeId,
        active_override: Option<(NodeId, NodeId)>,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let node = self
            .nodes
            .get(node_id)
            .ok_or(CommandError::MissingNode { node: node_id })?;
        let height = match node {
            Node::Panel(panel) => self
                .panel_instances
                .get(panel.panel_id)
                .ok_or(CommandError::MissingPanel {
                    panel: panel.panel_id,
                })?
                .constraints
                .preferred_size
                .height,
            Node::Tabs(tabs) if tabs.collapsed => metrics.effective_collapsed_group_height(),
            Node::Tabs(tabs) => {
                let active_child = active_override
                    .filter(|(group, _)| *group == node_id)
                    .map(|(_, panel_node)| panel_node)
                    .or_else(|| tabs.active_child())
                    .ok_or(CommandError::InvalidTarget {
                        reason: "Floating Tabs group has no active panel",
                    })?;
                let content_height = self.preferred_floating_node_height(
                    active_child,
                    floating_root,
                    active_override,
                    metrics,
                )?;
                let is_column_group =
                    node_id == floating_root || self.find_owner_column(node_id).is_some();
                let chrome_height = if is_column_group {
                    metrics.header_height.max(metrics.tab_strip_height)
                } else {
                    metrics.header_height + metrics.tab_strip_height
                };
                metrics.min_group_height.max(chrome_height + content_height)
            }
            Node::Column(column) if column.mode.is_rail() => {
                let slot_count = self.column_display_order(node_id)?.len();
                metrics.effective_collapsed_group_height() * slot_count as f32
            }
            Node::Column(column) => {
                let mut height = metrics.column_header_height;
                for group in &column.groups {
                    height += self.preferred_floating_node_height(
                        *group,
                        floating_root,
                        active_override,
                        metrics,
                    )?;
                }
                height += metrics.splitter_thickness
                    * column.groups.len().saturating_sub(1) as f32;
                height
            }
            Node::Split(split) => {
                let mut height = match split.axis {
                    Axis::X => 0.0_f32,
                    Axis::Y => {
                        metrics.splitter_thickness
                            * split.children.len().saturating_sub(1) as f32
                    }
                };
                for child in &split.children {
                    let child_height = self.preferred_floating_node_height(
                        *child,
                        floating_root,
                        active_override,
                        metrics,
                    )?;
                    match split.axis {
                        Axis::X => height = height.max(child_height),
                        Axis::Y => height += child_height,
                    }
                }
                height
            }
        };

        if !height.is_finite() || height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed preferred Floating node height is invalid",
            });
        }
        Ok(height)
    }

    /// Computes the minimum client width for resizing either horizontal edge
    /// while keeping all non-edge Columns at their current effective widths.
    ///
    /// The result is (left-edge minimum, right-edge minimum) for a Photoshop
    /// horizontal composition rooted at Split(X) of Columns.
    pub fn floating_horizontal_resize_minimum_widths(
        &self,
        group_id: crate::FloatingGroupId,
        metrics: &DockMetrics,
    ) -> Result<Option<(f32, f32)>, CommandError> {
        if self.profile != Profile::Photoshop {
            return Ok(None);
        }

        let floating = self
            .floating_groups
            .get(group_id)
            .copied()
            .ok_or(CommandError::MissingFloatingGroup { group: group_id })?;
        let root = self
            .dock_areas
            .get(floating.area)
            .ok_or(CommandError::InvalidTarget {
                reason: "floating group references a missing dock area",
            })?
            .root;
        let split = match self.nodes.get(root) {
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split.children.len() >= 2
                    && split.children.iter().all(|child| {
                        matches!(self.nodes.get(*child), Some(Node::Column(_)))
                    }) => split,
            _ => return Ok(None),
        };

        let rects = self
            .compute_floating_rects(group_id, metrics)
            .map_err(|_| CommandError::InvalidTarget {
                reason: "floating composition geometry is unavailable for edge minimum sizing",
            })?;
        let widths = split
            .children
            .iter()
            .map(|child| {
                rects
                    .columns
                    .iter()
                    .find(|column| column.column == *child)
                    .map(|column| column.rect.size.width)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "floating composition is missing a rendered Column rectangle",
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let minimums = split
            .children
            .iter()
            .map(|child| self.minimum_column_width(*child, metrics))
            .collect::<Result<Vec<_>, _>>()?;
        let splitter_width =
            metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;

        let fixed_after_first = widths.iter().skip(1).sum::<f32>();
        let fixed_before_last = widths.iter().take(widths.len() - 1).sum::<f32>();
        let left_minimum = minimums[0] + fixed_after_first + splitter_width;
        let right_minimum =
            fixed_before_last + minimums[minimums.len() - 1] + splitter_width;

        if !left_minimum.is_finite()
            || !right_minimum.is_finite()
            || left_minimum <= 0.0
            || right_minimum <= 0.0
        {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "edge-specific floating minimum width is invalid",
            });
        }

        Ok(Some((left_minimum, right_minimum)))
    }

    /// Computes floating minimum size while reusing per-node minimums in one
    /// immutable geometry operation. The caller owns the cache, so it cannot
    /// survive a mutation or accidentally mix layouts or metrics.
    pub(crate) fn minimum_floating_size_with_cache(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
        cache: &mut HashMap<NodeId, Size>,
    ) -> Result<Size, CommandError> {
        let minimum = match self.nodes.get(node_id) {
            Some(Node::Tabs(tabs)) if tabs.collapsed && !tabs.children.is_empty() => {
                let mut width = 0.0_f32;
                for child in &tabs.children {
                    width = width.max(
                        self.minimum_node_size_with_cache(*child, metrics, cache)?.width,
                    );
                }
                Size::new(width, metrics.effective_collapsed_group_height())
            }
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split.children.len() >= 2
                    && split.children.iter().all(|child| {
                        matches!(self.nodes.get(*child), Some(Node::Column(_)))
                    }) =>
            {
                // A Floating Split fills its client width with all Columns.
                // Keep the OS track-size floor aligned with the sum of their
                // minimum widths, including every intervening splitter.
                let mut minimum = Size::ZERO;
                for child in &split.children {
                    minimum.width += self.minimum_column_width(*child, metrics)?;
                    minimum.height = minimum.height.max(
                        self.minimum_floating_node_size_with_cache(
                            *child,
                            node_id,
                            metrics,
                            cache,
                        )?
                        .height,
                    );
                }
                minimum.width +=
                    metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
                minimum
            }
            Some(_) => self.minimum_floating_node_size_with_cache(
                node_id,
                node_id,
                metrics,
                cache,
            )?,
            None => return Err(CommandError::MissingNode { node: node_id }),
        };

        // A Floating root Tabs group shares the CompositionHeader row with
        // its tabs. Column roots already include their own header in the
        // recursive minimum above, so add this row only for a Tabs root.
        let composition_header_height = if matches!(self.nodes.get(node_id), Some(Node::Tabs(_))) {
            metrics.column_header_height
        } else {
            0.0
        };
        let minimum = Size::new(minimum.width, minimum.height + composition_header_height);

        if !minimum.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed floating minimum size overflowed",
            });
        }
        if minimum.width < 0.0 || minimum.height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed floating minimum size has invalid dimensions",
            });
        }
        Ok(minimum)
    }


}
