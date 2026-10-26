//! Workspace-load repairs for Photoshop column limits and singleton panels.



use super::*;



impl Layout {

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
    pub(super) fn repair_excess_photoshop_columns(&mut self, max_columns: usize) -> Vec<String> {
        if self.profile != Profile::Photoshop || max_columns == 0 {
            return Vec::new();
        }

        let mut columns_by_side = [Vec::<NodeId>::new(), Vec::<NodeId>::new()];
        for (side_index, columns_for_side) in columns_by_side.iter_mut().enumerate() {
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
            *columns_for_side = columns
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

}

