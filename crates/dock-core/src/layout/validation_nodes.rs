use super::helpers::*;
use super::*;

impl Layout {
    pub(super) fn validate_node_records(
        &self,
        metrics: &DockMetrics,
        metrics_are_valid: bool,
        errors: &mut Vec<ValidationError>,
    ) -> HashMap<PanelId, NodeId> {
        let mut panel_placements: HashMap<PanelId, NodeId> = HashMap::new();

        for (node_id, node) in self.nodes.iter() {
            match node {
                Node::Split(split) => {
                    if split.children.len() < 2 {
                        errors.push(ValidationError::InvalidSplit {
                            node: node_id,
                            reason: "must contain at least two children",
                        });
                    }
                    for reason in ratio_issues(&split.ratios, split.children.len()) {
                        errors.push(ValidationError::InvalidSplit {
                            node: node_id,
                            reason,
                        });
                    }
                }
                Node::Tabs(tabs) => {
                    if tabs.children.is_empty() {
                        errors.push(ValidationError::InvalidTabs {
                            node: node_id,
                            reason: "must contain at least one child",
                        });
                    }
                    if tabs.active_index >= tabs.children.len() {
                        errors.push(ValidationError::InvalidTabs {
                            node: node_id,
                            reason: "active_index is outside children",
                        });
                    }
                    if self.profile == Profile::Photoshop {
                        for child in &tabs.children {
                            if matches!(self.nodes.get(*child), Some(Node::Panel(_))) {
                                continue;
                            }
                            if self.nodes.contains_key(*child) {
                                errors.push(ValidationError::InvalidTabs {
                                    node: node_id,
                                    reason: "Photoshop Tabs children must be Panel nodes",
                                });
                            }
                        }
                    }
                }
                Node::Panel(panel) => {
                    if !self.panel_instances.contains_key(panel.panel_id) {
                        errors.push(ValidationError::MissingPanelInstance {
                            node: node_id,
                            panel: panel.panel_id,
                        });
                    }
                    if let Some(first_node) = panel_placements.get(&panel.panel_id) {
                        errors.push(ValidationError::DuplicatePanelPlacement {
                            panel: panel.panel_id,
                            first_node: *first_node,
                            second_node: node_id,
                        });
                    } else {
                        panel_placements.insert(panel.panel_id, node_id);
                    }
                }
                Node::Column(column) => {
                    if !column.width.is_finite() {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "width must be finite",
                        });
                    } else if column.width <= 0.0 {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "width must be positive",
                        });
                    } else if metrics_are_valid {
                        match column.mode {
                            ColumnMode::Expanded
                                if column.width + PIXEL_TOLERANCE < metrics.min_column_width =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "expanded width is below min_column_width",
                                });
                            }
                            ColumnMode::IconsAndLabels
                                if (column.width - metrics.min_column_width).abs()
                                    > PIXEL_TOLERANCE =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "IconsAndLabels width must equal min_column_width",
                                });
                            }
                            ColumnMode::IconsOnly
                                if (column.width - metrics.rail_width).abs() > PIXEL_TOLERANCE =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "IconsOnly width must equal rail_width",
                                });
                            }
                            _ => {}
                        }
                    }

                    if let Some(width) = column.last_expanded_width
                        && (!width.is_finite() || width <= 0.0)
                    {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "last_expanded_width must be finite and positive",
                        });
                    }

                    if column.groups.is_empty() {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "must contain at least one group before normalization",
                        });
                    }
                    for reason in ratio_issues(&column.group_ratios, column.groups.len()) {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason,
                        });
                    }
                    if self.profile == Profile::Photoshop {
                        for group in &column.groups {
                            if matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
                                continue;
                            }
                            if self.nodes.contains_key(*group) {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "Photoshop Column groups must be Tabs nodes",
                                });
                            }
                        }
                    }
                }
            }
        }
        panel_placements
    }

    pub(super) fn validate_panel_instances(
        &self,
        panel_placements: &HashMap<PanelId, NodeId>,
        errors: &mut Vec<ValidationError>,
    ) {
        let mut singleton_by_key: HashMap<crate::PanelKey, (PanelId, bool)> = HashMap::new();
        for (panel_id, instance) in self.panel_instances.iter() {
            if let Err(reason) = instance.validate() {
                errors.push(ValidationError::InvalidPanelInstance {
                    panel: panel_id,
                    reason,
                });
            }
            if !panel_placements.contains_key(&panel_id) {
                errors.push(ValidationError::UnplacedPanelInstance { panel: panel_id });
            }

            let key = instance.key.clone();
            let singleton = instance.constraints.singleton;
            if let Some((first_id, first_singleton)) = singleton_by_key.get_mut(&key) {
                if singleton || *first_singleton {
                    errors.push(ValidationError::DuplicateSingletonPanel {
                        key,
                        first: *first_id,
                        second: panel_id,
                    });
                }
                *first_singleton |= singleton;
            } else {
                singleton_by_key.insert(key, (panel_id, singleton));
            }
        }
    }
}
