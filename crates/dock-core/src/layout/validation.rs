use super::helpers::*;
use super::*;

const MAX_LAYOUT_TREE_DEPTH: usize = 256;

impl Layout {
    /// Validates the entire model and returns every detected issue.
    ///
    /// Metrics are supplied explicitly so callers and tests use the same
    /// thresholds as the core algorithms. An empty layout is valid: the canvas
    /// always exists, while empty main dock areas are represented by None.
    pub fn validate(&self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let metrics_result = metrics.validate();
        if let Err(reason) = metrics_result {
            errors.push(ValidationError::InvalidMetrics { reason });
        }
        let metrics_are_valid = metrics_result.is_ok();

        if let Err(reason) = self.canvas.validate() {
            errors.push(ValidationError::InvalidCanvas {
                reason: geometry_error_reason(reason),
            });
        }
        let panel_placements =
            self.validate_node_records(metrics, metrics_are_valid, &mut errors);
        self.validate_panel_instances(&panel_placements, &mut errors);

        // Validate area records and discover the only legal entry points into
        // the tree: registered main sides and floating groups with a floating area.
        let mut registered_areas = HashSet::new();
        let mut roots = Vec::new();
        let mut photoshop_main_columns = 0usize;

        for (area_id, area) in self.dock_areas.iter() {
            if let Err(reason) = area.validate_metadata() {
                errors.push(ValidationError::InvalidDockArea {
                    area: area_id,
                    reason,
                });
            }
        }

        for (side, index) in [(DockSide::Left, 0usize), (DockSide::Right, 1usize)] {
            let Some(area_id) = self.main_areas[index] else {
                continue;
            };
            if !registered_areas.insert(area_id) {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "the same DockAreaId is assigned to more than one main side",
                });
            }

            let Some(area) = self.dock_areas.get(area_id) else {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "references a missing DockAreaId",
                });
                continue;
            };

            if area.kind != DockAreaKind::Main {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "main side must reference a Main DockArea",
                });
            }
            if area.side != Some(side) {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "DockArea side does not match its main-area slot",
                });
            }

            if self.nodes.contains_key(area.root) {
                roots.push(area.root);
                if self.profile == Profile::Photoshop
                    && area.kind == DockAreaKind::Main
                    && area.side == Some(side)
                {
                    photoshop_main_columns +=
                        validate_photoshop_main_root(&self.nodes, area_id, area.root, &mut errors);
                }
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: None,
                    missing: area.root,
                });
            }
        }

        let mut groups_by_area: HashMap<DockAreaId, Vec<FloatingGroupId>> = HashMap::new();
        let mut groups_by_z: HashMap<i32, FloatingGroupId> = HashMap::new();

        for (group_id, group) in self.floating_groups.iter() {
            if let Err(reason) = group.validate() {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: match reason {
                        GeometryError::NonFinite => "bounds contain a non-finite value",
                        GeometryError::NegativeDimension => {
                            "bounds dimensions must not be negative"
                        }
                        GeometryError::NonPositiveDimension => {
                            "bounds must have positive dimensions"
                        }
                        GeometryError::ArithmeticOverflow => "bounds edges overflow",
                        GeometryError::InconsistentConstraints => "invalid bounds",
                    },
                });
            }

            if let Some(first_id) = groups_by_z.insert(group.z_order, group_id) {
                errors.push(ValidationError::DuplicateFloatingZOrder {
                    first: first_id,
                    second: group_id,
                    z_order: group.z_order,
                });
            }

            let Some(area) = self.dock_areas.get(group.area) else {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "references a missing DockAreaId",
                });
                continue;
            };
            if area.kind != DockAreaKind::Floating {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "must reference a Floating DockArea",
                });
                continue;
            }

            registered_areas.insert(group.area);
            groups_by_area.entry(group.area).or_default().push(group_id);
            if self.nodes.contains_key(area.root) {
                roots.push(area.root);
                if self.profile == Profile::Photoshop {
                    validate_photoshop_floating_root(&self.nodes, group_id, area.root, &mut errors);
                }
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: None,
                    missing: area.root,
                });
            }
        }

        for (area_id, group_ids) in &groups_by_area {
            if group_ids.len() > 1 {
                errors.push(ValidationError::FloatingAreaReferencedMoreThanOnce {
                    area: *area_id,
                    first: group_ids[0],
                    second: group_ids[1],
                });
            }
        }

        for (area_id, _) in self.dock_areas.iter() {
            if !registered_areas.contains(&area_id) {
                errors.push(ValidationError::UnreferencedDockArea { area: area_id });
            }
        }

        if metrics_are_valid
            && self.profile == Profile::Photoshop
            && photoshop_main_columns > metrics.max_columns
        {
            errors.push(ValidationError::TooManyColumns {
                actual: photoshop_main_columns,
                max: metrics.max_columns,
            });
        }

        if let Some(flyout) = self.flyout {
            match self.nodes.get(flyout.column) {
                None => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing column node",
                }),
                Some(Node::Column(column)) => {
                    if !column.mode.is_rail() {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "flyout column must use a rail mode",
                        });
                    }
                    if !column
                        .groups
                        .iter()
                        .any(|root| self.is_descendant_or_same(*root, flyout.group))
                    {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "group is not a member of the referenced column",
                        });
                    }
                }
                Some(_) => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "column reference does not point to a Column node",
                }),
            }

            if !self.nodes.contains_key(flyout.group) {
                errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing group node",
                });
            }

            let panel_belongs_to_group = match self.nodes.get(flyout.group) {
                Some(Node::Tabs(tabs)) => tabs.children.iter().any(|child| {
                    matches!(
                        self.nodes.get(*child),
                        Some(Node::Panel(panel)) if panel.panel_id == flyout.panel
                    )
                }),
                _ => false,
            };
            if !panel_belongs_to_group {
                errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "panel is not a child of the flyout group",
                });
            }
        }

        // Bound recursive validation/normalization work for programmatically
        // built trees as well as serde-loaded DTOs. Longest-depth tracking also
        // lets us stop a cyclic graph before a recursive walk can exhaust stack.
        let mut colors = HashMap::<NodeId, u8>::new();
        let mut pending = Vec::<(NodeId, usize, bool)>::new();
        for start in roots.iter().copied().chain(self.nodes.keys()) {
            if colors.contains_key(&start) {
                continue;
            }
            pending.push((start, 1, false));
            while let Some((current, depth, exiting)) = pending.pop() {
                if exiting {
                    colors.insert(current, 2);
                    continue;
                }
                if depth > MAX_LAYOUT_TREE_DEPTH {
                    errors.push(ValidationError::TreeTooDeep {
                        max_depth: MAX_LAYOUT_TREE_DEPTH,
                    });
                    return Err(errors);
                }
                match colors.get(&current).copied() {
                    Some(1) | Some(2) => continue,
                    _ => {}
                }
                colors.insert(current, 1);
                pending.push((current, depth, true));
                if let Some(node) = self.nodes.get(current) {
                    pending.extend(
                        node.children()
                            .iter()
                            .rev()
                            .copied()
                            .map(|child| (child, depth + 1, false)),
                    );
                }
            }
        }

        // Count all tree-edge references to find reused children, then mark
        // reachability only from registered area roots. These are deliberately
        // separate passes: cycles and missing links in an orphaned component
        // must still be reported instead of hidden by an early return.
        let mut incoming: HashMap<NodeId, usize> = HashMap::new();
        for root in &roots {
            *incoming.entry(*root).or_default() += 1;
        }
        let mut graph_visited = HashSet::new();
        let mut visiting = HashSet::new();
        for (node_id, _) in self.nodes.iter() {
            inspect_graph(
                &self.nodes,
                node_id,
                None,
                &mut graph_visited,
                &mut visiting,
                &mut incoming,
                &mut errors,
            );
        }

        for (node_id, _) in self.nodes.iter() {
            if let Some(references) = incoming.get(&node_id)
                && *references > 1
            {
                errors.push(ValidationError::MultipleParents {
                    node: node_id,
                    references: *references,
                });
            }
        }

        let mut reachable = HashSet::new();
        for root in &roots {
            mark_reachable(&self.nodes, *root, &mut reachable);
        }
        for (node_id, _) in self.nodes.iter() {
            if !reachable.contains(&node_id) {
                errors.push(ValidationError::UnreachableNode { node: node_id });
            }
        }

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

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
