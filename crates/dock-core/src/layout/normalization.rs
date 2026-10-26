use super::helpers::*;
use super::*;

impl Layout {
    /// Test-only wrapper for the C3 normalization contract. Production mutations
    /// call normalize_in_place on a private candidate inside Layout::apply.
    #[cfg(test)]
    pub(super) fn normalize(&mut self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut candidate = self.clone();
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(())
    }

    /// Crate-visible entry point for tests in sibling modules. This does not
    /// expose normalization in production builds or to downstream UI code.
    #[cfg(test)]
    pub(crate) fn normalize_for_test(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<(), Vec<ValidationError>> {
        self.normalize(metrics)
    }

    #[cfg(feature = "serde")]
    fn repair_duplicate_singleton_panels(&mut self) -> Vec<String> {
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
            if !self.repair_duplicate_singletons_in_node(
                area.root,
                &mut first_by_key,
                &mut notes,
            ) {
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
            if !self.repair_duplicate_singletons_in_node(
                area.root,
                &mut first_by_key,
                &mut notes,
            ) {
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
    fn repair_excess_photoshop_columns(&mut self, max_columns: usize) -> Vec<String> {
        if self.profile != Profile::Photoshop || max_columns == 0 {
            return Vec::new();
        }

        let mut columns_by_side = [Vec::<NodeId>::new(), Vec::<NodeId>::new()];
        for side_index in 0..2 {
            let Some(area_id) = self.main_areas[side_index] else {
                continue;
            };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                return Vec::new();
            };

            let columns = match self.nodes.get(area.root) {
                Some(Node::Column(_)) => vec![area.root],
                Some(Node::Split(split))
                    if split.axis == Axis::X
                        && split
                            .children
                            .iter()
                            .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
                {
                    split.children.clone()
                }
                _ => return Vec::new(),
            };

            // Completely empty columns will be removed by normal normalization.
            // Do not spend the available column budget on data that has no panel.
            columns_by_side[side_index] = columns
                .into_iter()
                .filter(|column| self.subtree_contains_panel(*column))
                .collect();
        }

        let total_columns: usize = columns_by_side.iter().map(Vec::len).sum();
        if total_columns <= max_columns {
            return Vec::new();
        }

        let active_sides = columns_by_side
            .iter()
            .filter(|columns| !columns.is_empty())
            .count();
        let mut retained = HashSet::<NodeId>::new();

        // Preserve a column on each populated side whenever the new limit allows
        // it, then use any remaining slots in stable left-to-right order.
        if max_columns >= active_sides {
            for columns in &columns_by_side {
                if let Some(first) = columns.first() {
                    retained.insert(*first);
                }
            }
            let mut remaining = max_columns - active_sides;
            for columns in &columns_by_side {
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
            for columns in &columns_by_side {
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
        for columns in &columns_by_side {
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
                if let Some((_, sources)) =
                    merges.iter_mut().find(|(existing_target, _)| *existing_target == target)
                {
                    sources.push(source);
                } else {
                    merges.push((target, vec![source]));
                }
            }
        }

        let mut notes = Vec::new();
        let mut removed_columns = HashSet::<NodeId>::new();
        for (target, sources) in &merges {
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

        if removed_columns.is_empty() {
            return notes;
        }

        // Remove the old column wrappers from their main roots after their groups
        // have been adopted by the retained columns.
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

        for source in removed_columns {
            self.nodes.remove(source);
        }

        notes
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


    pub(super) fn normalize_in_place(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<(), Vec<ValidationError>> {
        let existing_errors = self.validate(metrics).err().unwrap_or_default();
        let blockers: Vec<_> = existing_errors
            .into_iter()
            .filter(|error| !is_normalizable_validation_error(error))
            .collect();
        if !blockers.is_empty() {
            return Err(blockers);
        }

        // Main roots are normalized first. An empty main root removes its
        // DockArea and clears the corresponding side; it never leaves an empty
        // invisible area in the arena.
        let main_areas = self.main_areas;
        for (index, area_id) in main_areas.into_iter().enumerate() {
            let Some(area_id) = area_id else {
                continue;
            };
            let side = if index == 0 { DockSide::Left } else { DockSide::Right };
            let Some(area) = self.dock_areas.get(area_id).copied() else {
                return Err(vec![ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "references a missing DockAreaId",
                }]);
            };

            match self.normalize_node(area.root) {
                Some(normalized_root) => {
                    let Some(area) = self.dock_areas.get_mut(area_id) else {
                        return Err(vec![ValidationError::InvalidMainArea {
                            area: area_id,
                            side,
                            reason: "references a missing DockAreaId",
                        }]);
                    };
                    area.root = normalized_root;
                }
                None => {
                    self.dock_areas.remove(area_id);
                    self.main_areas[index] = None;
                }
            }
        }

        // Floating roots are normalized the same way. Empty floating roots
        // remove both the FloatingGroup and its associated DockArea.
        let floating_ids: Vec<_> = self.floating_groups.keys().collect();
        for group_id in floating_ids {
            let Some(group) = self.floating_groups.get(group_id).copied() else {
                continue;
            };
            let Some(area) = self.dock_areas.get(group.area).copied() else {
                return Err(vec![ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "references a missing DockAreaId",
                }]);
            };

            match self.normalize_node(area.root) {
                Some(normalized_root) => {
                    let Some(area) = self.dock_areas.get_mut(group.area) else {
                        return Err(vec![ValidationError::InvalidFloatingGroup {
                            group: group_id,
                            reason: "references a missing DockAreaId",
                        }]);
                    };
                    area.root = normalized_root;
                }
                None => {
                    self.floating_groups.remove(group_id);
                    self.dock_areas.remove(group.area);
                }
            }
        }

        // Flyouts are transient. If normalization removed or changed their
        // target, close the flyout instead of retaining stale runtime handles.
        let keep_flyout = self.flyout.is_some_and(|flyout| {
            let belongs_to_main_dock = [DockSide::Left, DockSide::Right]
                .into_iter()
                .filter_map(|side| self.main_area(side).map(|area| area.root))
                .any(|root| self.is_descendant_or_same(root, flyout.column));
            belongs_to_main_dock
                && matches!(
                    self.nodes.get(flyout.column),
                    Some(Node::Column(column))
                        if column.mode.is_rail()
                            && column.groups.iter().any(|root| self.is_descendant_or_same(*root, flyout.group))
                            && matches!(
                                self.nodes.get(flyout.group),
                                Some(Node::Tabs(tabs)) if tabs.children.iter().any(|child| matches!(
                                    self.nodes.get(*child),
                                    Some(Node::Panel(panel)) if panel.panel_id == flyout.panel
                                ))
                            )
                )
        });
        if !keep_flyout {
            self.flyout = None;
        }

        self.validate(metrics)
    }

    /// Returns the normalized representative of a node, or None when the
    /// entire subtree is empty. Preflight validation guarantees that every
    /// referenced node exists and that the graph is acyclic and single-parent.
    pub(super) fn normalize_node(&mut self, node_id: NodeId) -> Option<NodeId> {
        let node = self.nodes.get(node_id)?.clone();
        match node {
            Node::Panel(_) => Some(node_id),
            Node::Tabs(mut tabs) => {
                let old_children = tabs.children;
                let old_active_index = tabs.active_index;
                let mut kept = Vec::with_capacity(old_children.len());

                for (old_index, child) in old_children.into_iter().enumerate() {
                    if let Some(normalized_child) = self.normalize_node(child) {
                        kept.push((old_index, normalized_child));
                    }
                }

                if kept.is_empty() {
                    self.nodes.remove(node_id);
                    return None;
                }

                tabs.active_index = kept
                    .iter()
                    .position(|(old_index, _)| *old_index == old_active_index)
                    .unwrap_or_else(|| old_active_index.min(kept.len() - 1));
                tabs.children = kept.into_iter().map(|(_, child)| child).collect();
                let Some(node) = self.nodes.get_mut(node_id) else {
                    return None;
                };
                *node = Node::Tabs(tabs);
                Some(node_id)
            }
            Node::Split(mut split) => {
                let old_children = split.children;
                let old_ratios = split.ratios;
                let mut kept = Vec::with_capacity(old_children.len());

                for (old_index, child) in old_children.into_iter().enumerate() {
                    if let Some(normalized_child) = self.normalize_node(child) {
                        kept.push((normalized_child, old_ratios.get(old_index).copied()));
                    }
                }

                match kept.len() {
                    0 => {
                        self.nodes.remove(node_id);
                        None
                    }
                    1 => {
                        let (only_child, _) = kept[0];
                        self.nodes.remove(node_id);
                        Some(only_child)
                    }
                    _ => {
                        split.children = kept.iter().map(|(child, _)| *child).collect();
                        split.ratios =
                            normalized_ratios(kept.iter().map(|(_, ratio)| *ratio).collect());
                        let Some(node) = self.nodes.get_mut(node_id) else {
                            return None;
                        };
                        *node = Node::Split(split);
                        Some(node_id)
                    }
                }
            }
            Node::Column(mut column) => {
                let old_groups = column.groups;
                let old_ratios = column.group_ratios;
                let mut kept = Vec::with_capacity(old_groups.len());

                for (old_index, group) in old_groups.into_iter().enumerate() {
                    if let Some(normalized_group) = self.normalize_node(group) {
                        kept.push((normalized_group, old_ratios.get(old_index).copied()));
                    }
                }

                if kept.is_empty() {
                    self.nodes.remove(node_id);
                    return None;
                }

                // Deliberately do not collapse a Column that has exactly one group.
                column.groups = kept.iter().map(|(group, _)| *group).collect();
                column.group_ratios =
                    normalized_ratios(kept.iter().map(|(_, ratio)| *ratio).collect());
                let Some(node) = self.nodes.get_mut(node_id) else {
                    return None;
                };
                *node = Node::Column(column);
                Some(node_id)
            }
        }
    }

    /// Internal constructor used by pure core factories and loading/migration.
    ///
    /// It builds a complete value rather than exposing mutable arena access to
    /// consumers. Public persistent mutation remains command-driven.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        nodes: SlotMap<NodeId, Node>,
        panel_instances: SlotMap<PanelId, PanelInstance>,
        dock_areas: SlotMap<DockAreaId, DockArea>,
        main_areas: [Option<DockAreaId>; 2],
        floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
        canvas: CanvasState,
        profile: Profile,
        version: u64,
        chrome: ChromeMode,
    ) -> Self {
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
            version,
            chrome,
            flyout: None,
            flyout_auto_collapse: true,
            history_identity: Arc::new(()),
            flyout_widths: HashMap::new(),
            flyout_width_order: VecDeque::new(),
            validation_cache: ValidationCache::default(),
        }
    }
}
