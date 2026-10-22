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
        let mut ratios = vec![0.0; column.groups.len()];
        if expanded_height <= 0.0 {
            return Ok(column.group_ratios.clone());
        }
        for (index, group) in column.groups.iter().copied().enumerate() {
            if !self.group_is_collapsed(group) {
                ratios[index] = heights[index] / expanded_height;
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
        metrics: &DockMetrics,
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
        let mut maximum = None;
        // Groups share one column width; the narrowest finite group limit wins.
        for group in &column.groups {
            maximum = minimum_width_limit(
                maximum,
                self.maximum_content_width(*group, metrics)?,
            );
        }
        Ok(maximum)
    }

    fn maximum_content_width(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Option<f32>, CommandError> {
        let node = self
            .nodes
            .get(node_id)
            .ok_or(CommandError::MissingNode { node: node_id })?;
        match node {
            Node::Panel(panel) => Ok(self
                .panel_instances
                .get(panel.panel_id)
                .ok_or(CommandError::MissingPanel {
                    panel: panel.panel_id,
                })?
                .constraints
                .max_size
                .map(|size| size.width)),
            Node::Tabs(tabs) => {
                let mut maximum = None;
                for child in &tabs.children {
                    maximum = minimum_width_limit(
                        maximum,
                        self.maximum_content_width(*child, metrics)?,
                    );
                }
                Ok(maximum)
            }
            Node::Split(split) if split.axis == Axis::X => {
                let mut width = 0.0_f32;
                for child in &split.children {
                    let Some(child_maximum) = self.maximum_content_width(*child, metrics)? else {
                        // One unbounded child makes the combined horizontal
                        // limit unbounded as well.
                        return Ok(None);
                    };
                    width += child_maximum;
                }
                width += metrics.splitter_thickness
                    * split.children.len().saturating_sub(1) as f32;
                if width.is_finite() {
                    Ok(Some(width))
                } else {
                    Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "maximum panel-content width overflowed",
                    })
                }
            }
            Node::Split(split) => {
                let mut maximum = None;
                for child in &split.children {
                    maximum = minimum_width_limit(
                        maximum,
                        self.maximum_content_width(*child, metrics)?,
                    );
                }
                Ok(maximum)
            }
            Node::Column(column) if column.mode.is_rail() => Ok(Some(column.width)),
            Node::Column(column) => {
                let mut maximum = None;
                for group in &column.groups {
                    maximum = minimum_width_limit(
                        maximum,
                        self.maximum_content_width(*group, metrics)?,
                    );
                }
                Ok(maximum)
            }
        }
    }

    pub(super) fn maximum_column_width(
        &self,
        column_id: NodeId,
        metrics: &DockMetrics,
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
        self.maximum_expanded_column_width(column_id, metrics)
    }

    /// Computes a floating window's minimum size.
    ///
    /// A collapsed root Tabs group is rendered as a strip, but its floating
    /// window must still be large enough for the minimum size of its panel
    /// content. This keeps ResizeFloating from shrinking the window below the
    /// underlying panel constraints merely because the group is collapsed.
    pub(crate) fn minimum_floating_size(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
        let Some(Node::Tabs(tabs)) = self.nodes.get(node_id) else {
            return self.minimum_node_size(node_id, metrics);
        };
        if !tabs.collapsed || tabs.children.is_empty() {
            return self.minimum_node_size(node_id, metrics);
        }

        let mut minimum = Size::ZERO;
        for child in &tabs.children {
            let child_size = self.minimum_node_size(*child, metrics)?;
            minimum.width = minimum.width.max(child_size.width);
            minimum.height = minimum.height.max(child_size.height);
        }
        if !minimum.is_finite() || minimum.width < 0.0 || minimum.height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed floating minimum size is non-finite or has invalid dimensions",
            });
        }
        Ok(minimum)
    }

    /// Computes the minimum layout size using the same rules as geometry.
    /// Expanded Tabs include their header and strip; Split/Column stacks include
    /// splitter gaps; Expanded Columns retain their configured width.
    pub(crate) fn minimum_node_size(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
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
                    let child_size = self.minimum_node_size(*child, metrics)?;
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
                    let child_size = self.minimum_node_size(*child, metrics)?;
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
                    height += self.minimum_node_size(*group, metrics)?.height;
                }
                height += metrics.splitter_thickness
                    * column.groups.len().saturating_sub(1) as f32;
                Size::new(column.width, height)
            }
        };

        if !size.is_finite() || size.width < 0.0 || size.height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "size",
                reason: "computed minimum size is non-finite or has invalid dimensions",
            });
        }
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

fn minimum_width_limit(current: Option<f32>, candidate: Option<f32>) -> Option<f32> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.min(candidate)),
        (Some(current), None) => Some(current),
        (None, Some(candidate)) => Some(candidate),
        (None, None) => None,
    }
}
