use super::*;

impl Layout {
    #[cfg(feature = "serde")]
    pub(super) fn repair_workspace_column_widths(
        &mut self,
        metrics: &DockMetrics,
        repair_notes: &mut Vec<String>,
    ) -> Result<(), Vec<ValidationError>> {
        // Column presentation mode uniquely determines rail width. Correcting
        // a stale width is deterministic; an invalid expanded width is repaired
        // only when it is finite and positive, since otherwise intent is unclear.
        for (node_id, node) in self.nodes.iter_mut() {
            let Node::Column(column) = node else {
                continue;
            };
            match column.mode {
                ColumnMode::IconsOnly if column.width != metrics.rail_width => {
                    repair_notes.push(format!(
                        "column {node_id:?}: IconsOnly width reset from {} to {}",
                        column.width, metrics.rail_width
                    ));
                    column.width = metrics.rail_width;
                }
                ColumnMode::IconsAndLabels if column.width != metrics.min_column_width => {
                    repair_notes.push(format!(
                        "column {node_id:?}: IconsAndLabels width reset from {} to {}",
                        column.width, metrics.min_column_width
                    ));
                    column.width = metrics.min_column_width;
                }
                ColumnMode::Expanded if column.width.is_finite() && column.width > 0.0 => {
                    if column.width < metrics.min_column_width {
                        repair_notes.push(format!(
                            "column {node_id:?}: expanded width raised from {} to minimum {}",
                            column.width, metrics.min_column_width
                        ));
                        column.width = metrics.min_column_width;
                    }
                    if column
                        .last_expanded_width
                        .is_none_or(|width| !width.is_finite() || width <= 0.0)
                    {
                        repair_notes.push(format!(
                            "column {node_id:?}: missing or invalid last_expanded_width restored from current width {}",
                            column.width
                        ));
                        column.last_expanded_width = Some(column.width);
                    }
                }
                _ => {}
            }
        }

        // Repair finite Expanded widths against the content's complete min/max
        // contract before the normalized layout is validated or exposed.
        let column_ids = self
            .nodes
            .iter()
            .filter_map(|(node_id, node)| matches!(node, Node::Column(_)).then_some(node_id))
            .collect::<Vec<_>>();
        for node_id in column_ids {
            let Some(Node::Column(column)) = self.nodes.get(node_id).cloned() else {
                continue;
            };
            if column.mode != ColumnMode::Expanded
                || !column.width.is_finite()
                || column.width <= 0.0
            {
                continue;
            }

            let minimum = self
                .minimum_expanded_column_width(node_id, metrics)
                .map_err(|_| {
                    vec![ValidationError::InvalidColumn {
                        node: node_id,
                        reason: "cannot compute minimum width required by expanded contents",
                    }]
                })?;
            let maximum = self
                .maximum_expanded_column_width(node_id)
                .map_err(|_| {
                    vec![ValidationError::InvalidColumn {
                        node: node_id,
                        reason: "cannot compute maximum width allowed by expanded contents",
                    }]
                })?;
            if maximum.is_some_and(|maximum| maximum < minimum) {
                return Err(vec![ValidationError::InvalidColumn {
                    node: node_id,
                    reason: "minimum width required by expanded contents exceeds panel maximum width",
                }]);
            }
            let upper = maximum.unwrap_or(f32::MAX);
            let repaired_width = column.width.max(minimum).min(upper);
            if repaired_width != column.width {
                let note = if repaired_width > column.width {
                    format!(
                        "column {node_id:?}: width raised to content minimum from {} to {}",
                        column.width, repaired_width
                    )
                } else {
                    format!(
                        "column {node_id:?}: width reduced to panel maximum from {} to {}",
                        column.width, repaired_width
                    )
                };
                repair_notes.push(note);
            }

            let repaired_last_expanded_width = match column.last_expanded_width {
                Some(width) if width.is_finite() && width > 0.0 => {
                    let repaired = width.max(minimum).min(upper);
                    if repaired != width {
                        repair_notes.push(format!(
                            "column {node_id:?}: last_expanded_width clamped from {width} to {repaired}"
                        ));
                    }
                    Some(repaired)
                }
                _ => {
                    repair_notes.push(format!(
                        "column {node_id:?}: missing or invalid last_expanded_width restored from current width {repaired_width}"
                    ));
                    Some(repaired_width)
                }
            };

            if let Some(Node::Column(value)) = self.nodes.get_mut(node_id) {
                value.width = repaired_width;
                value.last_expanded_width = repaired_last_expanded_width;
            }
        }

        Ok(())
    }
}
