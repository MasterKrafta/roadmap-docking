use super::helpers::{is_normalizable_validation_error, normalized_ratios};

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

        self.normalize_main_areas()?;

        self.normalize_floating_areas()?;

        self.close_invalid_flyout();

        self.validate(metrics)
    }

pub(super) fn normalize_main_areas(&mut self) -> Result<(), Vec<ValidationError>> {
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
        Ok(())
    }

    pub(super) fn normalize_floating_areas(&mut self) -> Result<(), Vec<ValidationError>> {
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
        Ok(())
    }

    pub(super) fn close_invalid_flyout(&mut self) {
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
                let node = self.nodes.get_mut(node_id)?;
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
                        let node = self.nodes.get_mut(node_id)?;
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
                let node = self.nodes.get_mut(node_id)?;
                *node = Node::Column(column);
                Some(node_id)
            }
        }
    }

}

