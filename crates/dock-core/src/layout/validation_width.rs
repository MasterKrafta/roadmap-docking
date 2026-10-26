use super::*;

impl Layout {
    pub(super) fn validate_expanded_column_widths(
        &self,
        metrics: &DockMetrics,
        metrics_are_valid: bool,
        errors: &mut Vec<ValidationError>,
    ) {
        // Only recurse into width constraints after graph validation succeeds;
        // minimum-size traversal assumes an acyclic, fully linked node tree.
        if errors.is_empty() && metrics_are_valid {
            for (node_id, node) in self.nodes.iter() {
                let Node::Column(column) = node else {
                    continue;
                };
                if column.mode != ColumnMode::Expanded {
                    continue;
                }

                let minimum = match self.minimum_expanded_column_width(node_id, metrics) {
                    Ok(minimum) => minimum,
                    Err(_) => {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "cannot compute minimum width required by expanded contents",
                        });
                        continue;
                    }
                };
                let maximum = match self.maximum_expanded_column_width(node_id) {
                    Ok(maximum) => maximum,
                    Err(_) => {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "cannot compute maximum width allowed by expanded contents",
                        });
                        continue;
                    }
                };
                if maximum.is_some_and(|maximum| maximum < minimum) {
                    errors.push(ValidationError::InvalidColumn {
                        node: node_id,
                        reason: "minimum width required by expanded contents exceeds panel maximum width",
                    });
                } else if column.width < minimum {
                    errors.push(ValidationError::InvalidColumn {
                        node: node_id,
                        reason: "expanded width is below minimum width required by its contents",
                    });
                }
            }
        }
    }
}
