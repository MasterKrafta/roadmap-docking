use super::*;

impl Layout {
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
