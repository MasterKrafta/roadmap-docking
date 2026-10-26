use super::*;

impl Layout {
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
}
