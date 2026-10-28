//! Workspace loading, export, floating-bound adjustment, and load repairs.

use super::*;

use crate::{WorkspaceFile, WorkspaceFileError};

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

        candidate.repair_workspace_column_widths(metrics, &mut repair_notes)?;

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

    /// Creates a validated workspace DTO suitable for the public JSON boundary.
    pub fn to_workspace_file(
        &self,
        metrics: &DockMetrics,
    ) -> Result<WorkspaceFile, WorkspaceFileError> {
        WorkspaceFile::from_layout(self, metrics)
    }

    #[cfg(feature = "serde")]
    fn subtree_contains_panel(&self, root: NodeId) -> bool {
        fn contains_panel(layout: &Layout, node_id: NodeId, visited: &mut HashSet<NodeId>) -> bool {
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
    pub(super) fn repair_excess_photoshop_columns(&mut self, max_columns: usize) -> Vec<String> {
        if self.profile != Profile::Photoshop || max_columns == 0 {
            return Vec::new();
        }

        let Some(columns_by_side) = self.photoshop_main_columns_by_side() else {
            return Vec::new();
        };
        let total_columns: usize = columns_by_side.iter().map(Vec::len).sum();
        if total_columns <= max_columns {
            return Vec::new();
        }

        let retained = Self::select_photoshop_columns_to_keep(&columns_by_side, max_columns);
        let merges = Self::plan_photoshop_column_merges(&columns_by_side, &retained);
        let (notes, removed_columns) = self.merge_photoshop_column_groups(&merges, max_columns);
        if removed_columns.is_empty() {
            return notes;
        }

        self.remove_photoshop_column_wrappers(&removed_columns);
        notes
    }

    /// Collects populated main-dock columns in stable left-to-right, then right-side order.
    fn photoshop_main_columns_by_side(&self) -> Option<[Vec<NodeId>; 2]> {
        let mut columns_by_side = [Vec::<NodeId>::new(), Vec::<NodeId>::new()];
        for (side_index, columns_for_side) in columns_by_side.iter_mut().enumerate() {
            let Some(area_id) = self.main_areas[side_index] else {
                continue;
            };
            let area = self.dock_areas.get(area_id).copied()?;

            let columns = match self.nodes.get(area.root) {
                Some(Node::Column(_)) => vec![area.root],
                Some(Node::Split(split))
                    if split.axis == Axis::X
                        && split.children.iter().all(|child| {
                            matches!(self.nodes.get(*child), Some(Node::Column(_)))
                        }) =>
                {
                    split.children.clone()
                }
                _ => return None,
            };

            // Empty columns are removed by normal normalization and do not consume the budget.
            *columns_for_side = columns
                .into_iter()
                .filter(|column| self.subtree_contains_panel(*column))
                .collect();
        }
        Some(columns_by_side)
    }

    /// Selects columns to preserve, keeping one per populated side when the limit permits.
    fn select_photoshop_columns_to_keep(
        columns_by_side: &[Vec<NodeId>; 2],
        max_columns: usize,
    ) -> HashSet<NodeId> {
        let active_sides = columns_by_side
            .iter()
            .filter(|columns| !columns.is_empty())
            .count();
        let mut retained = HashSet::<NodeId>::new();

        if max_columns >= active_sides {
            for columns in columns_by_side {
                if let Some(first) = columns.first() {
                    retained.insert(*first);
                }
            }
            let mut remaining = max_columns - active_sides;
            for columns in columns_by_side {
                for column in columns.iter().skip(1) {
                    if remaining == 0 {
                        break;
                    }
                    retained.insert(*column);
                    remaining -= 1;
                }
                if remaining == 0 {
                    break;
                }
            }
        } else {
            for columns in columns_by_side {
                for column in columns {
                    if retained.len() >= max_columns {
                        break;
                    }
                    retained.insert(*column);
                }
                if retained.len() >= max_columns {
                    break;
                }
            }
        }

        retained
    }

    /// Plans which retained column receives each removed column, preserving side affinity.
    fn plan_photoshop_column_merges(
        columns_by_side: &[Vec<NodeId>; 2],
        retained: &HashSet<NodeId>,
    ) -> Vec<(NodeId, Vec<NodeId>)> {
        let ordered_columns: Vec<_> = columns_by_side
            .iter()
            .flat_map(|columns| columns.iter().copied())
            .collect();
        let ordered_retained: Vec<_> = ordered_columns
            .iter()
            .copied()
            .filter(|column| retained.contains(column))
            .collect();
        let Some(fallback_target) = ordered_retained.last().copied() else {
            return Vec::new();
        };

        let mut merges: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
        for columns in columns_by_side {
            let side_retained: Vec<_> = columns
                .iter()
                .copied()
                .filter(|column| retained.contains(column))
                .collect();
            let target = side_retained.last().copied().unwrap_or(fallback_target);

            for source in columns
                .iter()
                .copied()
                .filter(|column| !retained.contains(column))
            {
                if let Some((_, sources)) = merges
                    .iter_mut()
                    .find(|(existing_target, _)| *existing_target == target)
                {
                    sources.push(source);
                } else {
                    merges.push((target, vec![source]));
                }
            }
        }

        merges
    }

    /// Merges groups into the selected destination columns without dropping panels.
    fn merge_photoshop_column_groups(
        &mut self,
        merges: &[(NodeId, Vec<NodeId>)],
        max_columns: usize,
    ) -> (Vec<String>, HashSet<NodeId>) {
        let mut notes = Vec::new();
        let mut removed_columns = HashSet::<NodeId>::new();
        for (target, sources) in merges {
            // Each source column contributes one equal-height block. Its own
            // group ratios are retained within that block, so no panel is lost.
            let members: Vec<_> = std::iter::once(*target)
                .chain(sources.iter().copied())
                .collect();
            let block_scale = 1.0 / members.len() as f32;
            let mut merged_groups = Vec::new();
            let mut merged_ratios = Vec::new();

            for column_id in &members {
                let Some(Node::Column(column)) = self.nodes.get(*column_id).cloned() else {
                    continue;
                };
                let ratios = helpers::normalized_ratios(
                    column
                        .groups
                        .iter()
                        .enumerate()
                        .map(|(index, _)| column.group_ratios.get(index).copied())
                        .collect(),
                );
                for (group, ratio) in column.groups.into_iter().zip(ratios) {
                    merged_groups.push(group);
                    merged_ratios.push(Some(ratio * block_scale));
                }
            }

            if let Some(Node::Column(column)) = self.nodes.get_mut(*target) {
                column.groups = merged_groups;
                column.group_ratios = helpers::normalized_ratios(merged_ratios);
            }

            for source in sources {
                removed_columns.insert(*source);
                notes.push(format!(
                    "Photoshop main layout: column {source:?} merged into column {target:?} because max_columns decreased to {max_columns}"
                ));
            }
        }

        (notes, removed_columns)
    }

    /// Removes obsolete column nodes from main roots after their groups have been adopted.
    fn remove_photoshop_column_wrappers(&mut self, removed_columns: &HashSet<NodeId>) {
        for side_index in 0..2 {
            let Some(area_id) = self.main_areas[side_index] else {
                continue;
            };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                continue;
            };

            match self.nodes.get(area.root).cloned() {
                Some(Node::Column(_)) if removed_columns.contains(&area.root) => {
                    self.nodes.remove(area.root);
                    self.dock_areas.remove(area_id);
                    self.main_areas[side_index] = None;
                }
                Some(Node::Split(mut split)) if split.axis == Axis::X => {
                    let original_children = split.children.clone();
                    let original_ratios = split.ratios.clone();
                    let kept: Vec<_> = original_children
                        .iter()
                        .copied()
                        .enumerate()
                        .filter(|(_, child)| !removed_columns.contains(child))
                        .collect();

                    if kept.len() == original_children.len() {
                        continue;
                    }

                    match kept.len() {
                        0 => {
                            self.nodes.remove(area.root);
                            self.dock_areas.remove(area_id);
                            self.main_areas[side_index] = None;
                        }
                        1 => {
                            self.nodes.remove(area.root);
                            if let Some(area) = self.dock_areas.get_mut(area_id) {
                                area.root = kept[0].1;
                            }
                        }
                        _ => {
                            split.children = kept.iter().map(|(_, child)| *child).collect();
                            split.ratios = helpers::normalized_ratios(
                                kept.iter()
                                    .map(|(index, _)| original_ratios.get(*index).copied())
                                    .collect(),
                            );
                            if let Some(node) = self.nodes.get_mut(area.root) {
                                *node = Node::Split(split);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        for source in removed_columns.iter().copied() {
            self.nodes.remove(source);
        }
    }

    pub(super) fn repair_duplicate_singleton_panels(&mut self) -> Vec<String> {
        let mut first_by_key = HashMap::<PanelKey, (PanelId, bool)>::new();
        let mut notes = Vec::new();

        // Traverse the same roots in the same order used while restoring the
        // workspace DTO: left, right, then floating groups in stored order.
        let main_areas = self.main_areas;
        for (index, area_id) in main_areas.into_iter().enumerate() {
            let Some(area_id) = area_id else {
                continue;
            };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                continue;
            };
            if !self.repair_duplicate_singletons_in_node(area.root, &mut first_by_key, &mut notes) {
                self.main_areas[index] = None;
                self.dock_areas.remove(area_id);
            }
        }

        let floating_ids: Vec<_> = self.floating_groups.keys().collect();
        for group_id in floating_ids {
            let Some(group) = self.floating_groups.get(group_id).copied() else {
                continue;
            };
            let Some(area) = self.dock_areas.get(group.area).copied() else {
                continue;
            };
            if !self.repair_duplicate_singletons_in_node(area.root, &mut first_by_key, &mut notes) {
                self.floating_groups.remove(group_id);
                self.dock_areas.remove(group.area);
            }
        }

        notes
    }

    #[cfg(feature = "serde")]
    fn repair_duplicate_singletons_in_node(
        &mut self,
        node_id: NodeId,
        first_by_key: &mut HashMap<PanelKey, (PanelId, bool)>,
        notes: &mut Vec<String>,
    ) -> bool {
        let Some(node) = self.nodes.get(node_id).cloned() else {
            return true;
        };

        match node {
            Node::Panel(panel) => {
                let Some(instance) = self.panel_instances.get(panel.panel_id).cloned() else {
                    return true;
                };
                let key = instance.key.clone();
                let singleton = instance.constraints.singleton;

                if let Some((first_id, first_singleton)) = first_by_key.get_mut(&key) {
                    if singleton || *first_singleton {
                        let first_id = *first_id;
                        notes.push(format!(
                            "singleton panel {key:?}: duplicate instance {:?} removed; first instance {first_id:?} kept",
                            panel.panel_id
                        ));
                        self.panel_instances.remove(panel.panel_id);
                        self.nodes.remove(node_id);
                        return false;
                    }
                    *first_singleton |= singleton;
                } else {
                    first_by_key.insert(key, (panel.panel_id, singleton));
                }
                true
            }
            Node::Split(mut split) => {
                let original_children = split.children.clone();
                let original_ratios = split.ratios.clone();
                let mut kept = Vec::with_capacity(original_children.len());

                for (index, child) in original_children.iter().copied().enumerate() {
                    if self.repair_duplicate_singletons_in_node(child, first_by_key, notes) {
                        kept.push((index, child));
                    }
                }

                if kept.len() != original_children.len() {
                    split.children = kept.iter().map(|(_, child)| *child).collect();
                    split.ratios = helpers::normalized_ratios(
                        kept.iter()
                            .map(|(index, _)| original_ratios.get(*index).copied())
                            .collect(),
                    );
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        *node = Node::Split(split);
                    }
                }
                true
            }
            Node::Tabs(mut tabs) => {
                let original_children = tabs.children.clone();
                let original_active_index = tabs.active_index;
                let mut kept = Vec::with_capacity(original_children.len());

                for (index, child) in original_children.iter().copied().enumerate() {
                    if self.repair_duplicate_singletons_in_node(child, first_by_key, notes) {
                        kept.push((index, child));
                    }
                }

                if kept.len() != original_children.len() {
                    tabs.active_index = if kept.is_empty() {
                        0
                    } else {
                        kept.iter()
                            .position(|(index, _)| *index == original_active_index)
                            .unwrap_or_else(|| original_active_index.min(kept.len() - 1))
                    };
                    tabs.children = kept.iter().map(|(_, child)| *child).collect();
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        *node = Node::Tabs(tabs);
                    }
                }
                true
            }
            Node::Column(mut column) => {
                let original_groups = column.groups.clone();
                let original_ratios = column.group_ratios.clone();
                let mut kept = Vec::with_capacity(original_groups.len());

                for (index, group) in original_groups.iter().copied().enumerate() {
                    if self.repair_duplicate_singletons_in_node(group, first_by_key, notes) {
                        kept.push((index, group));
                    }
                }

                if kept.len() != original_groups.len() {
                    column.groups = kept.iter().map(|(_, group)| *group).collect();
                    column.group_ratios = helpers::normalized_ratios(
                        kept.iter()
                            .map(|(index, _)| original_ratios.get(*index).copied())
                            .collect(),
                    );
                    if let Some(node) = self.nodes.get_mut(node_id) {
                        *node = Node::Column(column);
                    }
                }
                true
            }
        }
    }

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
                        reason:
                            ValidationReason::CannotComputeMinimumWidthRequiredByExpandedContents,
                    }]
                })?;
            let maximum = self.maximum_expanded_column_width(node_id).map_err(|_| {
                vec![ValidationError::InvalidColumn {
                    node: node_id,
                    reason: ValidationReason::CannotComputeMaximumWidthAllowedByExpandedContents,
                }]
            })?;
            if maximum.is_some_and(|maximum| maximum < minimum) {
                return Err(vec![ValidationError::InvalidColumn {
                    node: node_id,
                    reason: ValidationReason::MinimumWidthRequiredByExpandedContentsExceedsPanelMaximumWidth,
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
