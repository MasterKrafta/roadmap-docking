use std::collections::HashMap;

use super::helpers::*;
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

    pub(super) fn adjust_canvas_for_column_resize(
        &mut self,
        column_id: NodeId,
        width_delta: f32,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let Some((side, _)) = self.main_column_position(column_id) else {
            return Ok(());
        };
        let Some(bounds) = self.canvas.last_known_bounds else {
            // The UI geometry pass will compute the canvas remainder once it
            // has a measured parent viewport.
            return Ok(());
        };
        let new_width = bounds.size.width - width_delta;
        let new_origin = Point::new(
            if side == DockSide::Left {
                bounds.origin.x + width_delta
            } else {
                bounds.origin.x
            },
            bounds.origin.y,
        );
        if !new_width.is_finite() || new_width + PIXEL_TOLERANCE < metrics.min_canvas_width {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "column resize would leave the canvas below min_canvas_width",
            });
        }
        let new_bounds = Rect::new(new_origin, Size::new(new_width, bounds.size.height));
        new_bounds
            .validate_non_empty()
            .map_err(|error| CommandError::InvalidValue {
                field: "canvas_bounds",
                reason: geometry_error_reason(error),
            })?;
        self.canvas.last_known_bounds = Some(new_bounds);
        Ok(())
    }

    pub(super) fn reset_main_column_widths(
        &mut self,
        side: DockSide,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let columns = self.main_columns(side)?;
        if columns.len() < 2 {
            return Ok(());
        }

        let current_widths: Vec<f32> = columns
            .iter()
            .map(|id| match self.nodes.get(*id) {
                Some(Node::Column(column)) => Ok(column.width),
                _ => Err(CommandError::InvalidTarget {
                    reason: "main horizontal split contains a non-Column child",
                }),
            })
            .collect::<Result<_, _>>()?;
        let total_width: f32 = current_widths.iter().copied().sum();
        if !total_width.is_finite() || total_width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "total dock column width must be finite and positive",
            });
        }

        // Find the closest equal width that satisfies each column's min/max
        // constraints while preserving the dock's total width. Rails stay fixed
        // at their presentation width.
        let mut minimums = Vec::with_capacity(columns.len());
        let mut maximums = Vec::with_capacity(columns.len());
        for (index, id) in columns.iter().copied().enumerate() {
            let column = match self.nodes.get(id) {
                Some(Node::Column(column)) => column,
                _ => {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split contains a non-Column child",
                    });
                }
            };
            if column.mode.is_rail() {
                minimums.push(current_widths[index]);
                maximums.push(current_widths[index]);
            } else {
                let minimum = self.minimum_column_width(id, metrics)?;
                let maximum = self
                    .maximum_column_width(id)?
                    .unwrap_or(total_width)
                    .min(total_width);
                if !minimum.is_finite() || !maximum.is_finite() || maximum < minimum {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "column width constraints are invalid",
                    });
                }
                minimums.push(minimum);
                maximums.push(maximum);
            }
        }

        let minimum_total: f32 = minimums.iter().copied().sum();
        let maximum_total: f32 = maximums.iter().copied().sum();
        if total_width + 1.0e-3 < minimum_total || total_width > maximum_total + 1.0e-3 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "current dock width cannot satisfy column min/max constraints",
            });
        }

        // Sum(clamp(target, minimum, maximum)) is monotonic, so bisection finds
        // the equal-width target even when some columns hit a constraint.
        let mut low = 0.0_f32;
        let mut high = total_width;
        for _ in 0..48 {
            let target = low + (high - low) * 0.5;
            let allocated: f32 = minimums
                .iter()
                .zip(&maximums)
                .map(|(minimum, maximum)| target.clamp(*minimum, *maximum))
                .sum();
            if allocated < total_width {
                low = target;
            } else {
                high = target;
            }
        }

        let target = low + (high - low) * 0.5;
        let mut widths: Vec<f32> = minimums
            .iter()
            .zip(&maximums)
            .map(|(minimum, maximum)| target.clamp(*minimum, *maximum))
            .collect();
        let mut residual = total_width - widths.iter().copied().sum::<f32>();
        for index in 0..widths.len() {
            if residual.abs() <= 1.0e-3 {
                break;
            }
            let adjustment = if residual > 0.0 {
                residual.min(maximums[index] - widths[index])
            } else {
                residual.max(minimums[index] - widths[index])
            };
            widths[index] += adjustment;
            residual -= adjustment;
        }
        if residual.abs() > 1.0e-2 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "equalized column widths do not preserve total dock width",
            });
        }

        for (id, width) in columns.into_iter().zip(widths) {
            if let Some(Node::Column(column)) = self.nodes.get_mut(id)
                && !column.mode.is_rail()
            {
                column.width = width;
                column.last_expanded_width = Some(width);
            }
        }
        Ok(())
    }

    pub(super) fn minimum_column_width(
        &self,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        match column.mode {
            ColumnMode::IconsOnly => Ok(metrics.rail_width),
            ColumnMode::IconsAndLabels => Ok(metrics.min_column_width),
            ColumnMode::Expanded => self.minimum_expanded_column_width(column_id, metrics),
        }
    }

    pub(super) fn minimum_expanded_column_width(
        &self,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "expanded minimum width requires a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        let mut minimum = metrics.min_column_width;
        for group in &column.groups {
            // Collapsed geometry hides panel content, but the column must keep
            // enough width to expand that group without producing invalid rects.
            minimum = minimum.max(self.minimum_content_width(*group, metrics)?);
        }
        if minimum.is_finite() {
            Ok(minimum)
        } else {
            Err(CommandError::InvalidValue {
                field: "width",
                reason: "minimum column width overflowed",
            })
        }
    }

    fn minimum_content_width(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let node = self
            .nodes
            .get(node_id)
            .ok_or(CommandError::MissingNode { node: node_id })?;
        let width = match node {
            Node::Panel(panel) => self
                .panel_instances
                .get(panel.panel_id)
                .ok_or(CommandError::MissingPanel {
                    panel: panel.panel_id,
                })?
                .constraints
                .min_size
                .width,
            Node::Tabs(tabs) => {
                let mut width = 0.0_f32;
                for child in &tabs.children {
                    width = width.max(self.minimum_content_width(*child, metrics)?);
                }
                width
            }
            Node::Split(split) => {
                let mut width = 0.0_f32;
                for child in &split.children {
                    let child_width = self.minimum_content_width(*child, metrics)?;
                    match split.axis {
                        Axis::X => width += child_width,
                        Axis::Y => width = width.max(child_width),
                    }
                }
                if split.axis == Axis::X {
                    width += metrics.splitter_thickness
                        * split.children.len().saturating_sub(1) as f32;
                }
                width
            }
            Node::Column(column) if column.mode.is_rail() => {
                if column.mode == ColumnMode::IconsOnly {
                    metrics.rail_width
                } else {
                    metrics.min_column_width
                }
            }
            Node::Column(column) => {
                let mut width = column.width.max(metrics.min_column_width);
                for group in &column.groups {
                    width = width.max(self.minimum_content_width(*group, metrics)?);
                }
                width
            }
        };
        if width.is_finite() && width >= 0.0 {
            Ok(width)
        } else {
            Err(CommandError::InvalidValue {
                field: "width",
                reason: "minimum panel-content width overflowed",
            })
        }
    }

    pub(super) fn maximum_expanded_column_width(
        &self,
        column_id: NodeId,
    ) -> Result<Option<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "column max-width calculation requires a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        let mut maximum: Option<f32> = None;
        for group in &column.groups {
            for panel_node in self.panel_nodes_in_subtree(*group) {
                let Some(Node::Panel(panel)) = self.nodes.get(panel_node) else {
                    continue;
                };
                let instance =
                    self.panel_instances
                        .get(panel.panel_id)
                        .ok_or(CommandError::MissingPanel {
                            panel: panel.panel_id,
                        })?;
                if let Some(max_size) = instance.constraints.max_size {
                    maximum =
                        Some(maximum.map_or(max_size.width, |current| current.min(max_size.width)));
                }
            }
        }
        Ok(maximum)
    }

    pub(super) fn maximum_column_width(
        &self,
        column_id: NodeId,
    ) -> Result<Option<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "column max-width calculation requires a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if column.mode.is_rail() {
            return Ok(Some(column.width));
        }
        self.maximum_expanded_column_width(column_id)
    }

    /// Computes a floating window's minimum size, including reserved title rows.
    ///
    /// A collapsed root Tabs group is rendered as a strip, but its floating
    /// window must still fit the minimum panel content size. Composite roots
    /// also reserve a title row above collapsed first branches, matching the
    /// geometry emitted by compute_rects.
    pub(crate) fn minimum_floating_size(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
        self.minimum_floating_size_with_cache(node_id, metrics, &mut HashMap::new())
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
                let mut minimum = Size::ZERO;
                for child in &tabs.children {
                    let child_size = self.minimum_node_size_with_cache(*child, metrics, cache)?;
                    minimum.width = minimum.width.max(child_size.width);
                    minimum.height = minimum.height.max(child_size.height);
                }
                minimum
            }
            Some(Node::Column(column))
                if column.groups.len() > 1
                    && self.floating_branch_starts_collapsed(node_id) =>
            {
                let base = self.minimum_node_size_with_cache(node_id, metrics, cache)?;
                Size::new(base.width, base.height + metrics.header_height)
            }
            Some(Node::Split(split)) if split.axis == Axis::Y && self.floating_branch_starts_collapsed(node_id) => {
                let base = self.minimum_node_size_with_cache(node_id, metrics, cache)?;
                Size::new(base.width, base.height + metrics.header_height)
            }
            Some(Node::Split(split)) if split.axis == Axis::X => {
                let mut width = 0.0_f32;
                let mut height = 0.0_f32;
                for child in &split.children {
                    let child_size = self.minimum_node_size_with_cache(*child, metrics, cache)?;
                    width += child_size.width;
                    let title_reserve = if self.floating_branch_starts_collapsed(*child) {
                        metrics.header_height
                    } else {
                        0.0
                    };
                    height = height.max(child_size.height + title_reserve);
                }
                width += metrics.splitter_thickness
                    * split.children.len().saturating_sub(1) as f32;
                Size::new(width, height)
            }
            Some(_) => self.minimum_node_size_with_cache(node_id, metrics, cache)?,
            None => return Err(CommandError::MissingNode { node: node_id }),
        };

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

    /// Whether the first visible vertical branch starts with a collapsed group.
    ///
    /// Horizontal floating compositions reserve a window-title row above such
    /// branches. A vertical split follows its first child to identify that row.
    pub(crate) fn floating_branch_starts_collapsed(&self, node_id: NodeId) -> bool {
        match self.nodes.get(node_id) {
            Some(Node::Tabs(tabs)) => tabs.collapsed,
            Some(Node::Column(column)) => column.groups.first().is_some_and(|group| {
                matches!(self.nodes.get(*group), Some(Node::Tabs(tabs)) if tabs.collapsed)
            }),
            Some(Node::Split(split)) if split.axis == Axis::Y => split
                .children
                .first()
                .is_some_and(|child| self.floating_branch_starts_collapsed(*child)),
            _ => false,
        }
    }

    /// Computes the minimum layout size using the same rules as geometry.
    /// Expanded Tabs include their header and strip; Split/Column stacks include
    /// splitter gaps; Expanded Columns retain their configured width.
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
                let mut content_height = 0.0_f32;
                for child in &tabs.children {
                    let child_size = self.minimum_node_size_with_cache(*child, metrics, cache)?;
                    width = width.max(child_size.width);
                    content_height = content_height.max(child_size.height);
                }
                Size::new(
                    width,
                    metrics
                        .min_group_height
                        .max(metrics.header_height + metrics.tab_strip_height + content_height),
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
                let mut height = 0.0_f32;
                for group in &column.groups {
                    height += self.minimum_node_size_with_cache(*group, metrics, cache)?.height;
                }
                height += metrics.splitter_thickness
                    * column.groups.len().saturating_sub(1) as f32;
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

    pub(super) fn next_floating_z_order(&self) -> Result<i32, CommandError> {
        match self
            .floating_groups
            .values()
            .map(|group| group.z_order)
            .max()
        {
            Some(value) => value.checked_add(1).ok_or(CommandError::InvalidValue {
                field: "z_order",
                reason: "floating z-order overflow",
            }),
            None => Ok(0),
        }
    }
}
