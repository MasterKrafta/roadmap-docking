use super::*;

impl Layout {
    #[cfg(feature = "serde")]
    pub(crate) fn from_workspace_parts(parts: WorkspaceLayoutParts) -> Self {
        let WorkspaceLayoutParts {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            canvas,
            profile,
            chrome,
        } = parts;

        Self {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            last_dock_locations: HashMap::new(),
            last_dock_location_order: VecDeque::new(),
            canvas,
            profile,
            version: 0,
            chrome,
            flyout: None,
            flyout_auto_collapse: true,
            history_identity: Arc::new(()),
            flyout_widths: HashMap::new(),
            flyout_width_order: VecDeque::new(),
            validation_cache: ValidationCache::default(),
        }
    }

    #[cfg(feature = "serde")]
    pub(crate) fn clamp_floating_groups_for_workspace_load(
        &mut self,
        client_area: Rect,
        metrics: &DockMetrics,
        original_indices: &HashMap<FloatingGroupId, usize>,
    ) -> FloatingBoundsClampResult {
        let mut groups: Vec<_> = self
            .floating_groups
            .iter()
            .enumerate()
            .map(|(fallback_index, (id, group))| {
                (
                    original_indices.get(&id).copied().unwrap_or(fallback_index),
                    id,
                    group.z_order,
                )
            })
            .collect();
        groups.sort_by_key(|(original_index, _, z_order)| (*z_order, *original_index));

        let mut changes = Vec::new();
        for (index, id, _) in groups {
            let Some(group) = self.floating_groups.get(id).copied() else {
                return Err((index, crate::ComputeRectsError::InvalidLayoutState));
            };
            let Some(root) = self.dock_areas.get(group.area).map(|area| area.root) else {
                return Err((index, crate::ComputeRectsError::InvalidLayoutState));
            };
            let original = group.bounds;
            let clamped = crate::compute_rects::clamp_floating_bounds(
                self,
                root,
                original,
                client_area,
                metrics,
            )
            .map_err(|error| (index, error))?;

            if clamped != original {
                let Some(group) = self.floating_groups.get_mut(id) else {
                    return Err((index, crate::ComputeRectsError::InvalidLayoutState));
                };
                group.bounds = clamped;
                changes.push((index, original, clamped));
            }
        }
        Ok(changes)
    }

    #[cfg(feature = "serde")]
    fn subtree_contains_panel(&self, root: NodeId) -> bool {
        fn contains_panel(
            layout: &Layout,
            node_id: NodeId,
            visited: &mut HashSet<NodeId>,
        ) -> bool {
            if !visited.insert(node_id) {
                return false;
            }
            let Some(node) = layout.nodes.get(node_id) else {
                return false;
            };
            matches!(node, Node::Panel(_))
                || node
                    .children()
                    .iter()
                    .copied()
                    .any(|child| contains_panel(layout, child, visited))
        }

        contains_panel(self, root, &mut HashSet::new())
    }

    #[cfg(feature = "serde")]
    pub(crate) fn normalize_for_workspace_load(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<Option<Vec<String>>, Vec<ValidationError>> {
        let original_issues = self.validate(metrics).err().unwrap_or_default();
        let mut candidate = self.clone();
        let mut repair_notes = Vec::new();

        // A canvas measurement saved by an older release may be valid geometry
        // but too narrow for newer metrics. Drop the stale measurement so the host
        // can supply the current viewport instead of inventing a wider canvas.
        if let Some(bounds) = candidate.canvas.last_known_bounds()
            && bounds.validate_non_empty().is_ok()
            && bounds.size.width + PIXEL_TOLERANCE < metrics.min_canvas_width
        {
            repair_notes.push(format!(
                "canvas last_known_bounds cleared because stored width {} is below current min_canvas_width {}",
                bounds.size.width, metrics.min_canvas_width
            ));
            candidate.canvas = CanvasState::default();
        }

        repair_notes.extend(candidate.repair_duplicate_singleton_panels());
        repair_notes.extend(candidate.repair_excess_photoshop_columns(metrics.max_columns));
        repair_notes.extend(candidate.repair_duplicate_floating_z_orders());

        // Column presentation mode uniquely determines rail width. Correcting
        // a stale width is deterministic; an invalid expanded width is repaired
        // only when it is finite and positive, since otherwise intent is unclear.
        for (node_id, node) in candidate.nodes.iter_mut() {
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
        // contract before the normalized candidate is validated or exposed.
        let column_ids = candidate
            .nodes
            .iter()
            .filter_map(|(node_id, node)| matches!(node, Node::Column(_)).then_some(node_id))
            .collect::<Vec<_>>();
        for node_id in column_ids {
            let Some(Node::Column(column)) = candidate.nodes.get(node_id).cloned() else {
                continue;
            };
            if column.mode != ColumnMode::Expanded
                || !column.width.is_finite()
                || column.width <= 0.0
            {
                continue;
            }

            let minimum = candidate
                .minimum_expanded_column_width(node_id, metrics)
                .map_err(|_| {
                    vec![ValidationError::InvalidColumn {
                        node: node_id,
                        reason: "cannot compute minimum width required by expanded contents",
                    }]
                })?;
            let maximum = candidate
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

            if let Some(Node::Column(value)) = candidate.nodes.get_mut(node_id) {
                value.width = repaired_width;
                value.last_expanded_width = repaired_last_expanded_width;
            }
        }

        let mut diagnostic_issues: Vec<String> =
            original_issues.iter().map(ToString::to_string).collect();
        diagnostic_issues.extend(repair_notes);

        if original_issues.is_empty() && diagnostic_issues.is_empty() {
            return Ok(None);
        }
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(Some(diagnostic_issues))
    }

}
