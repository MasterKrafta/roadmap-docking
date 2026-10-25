use super::*;

impl Layout {
    pub(super) fn main_columns(&self, side: DockSide) -> Result<Vec<NodeId>, CommandError> {
        let Some(area) = self.main_area(side) else {
            return Ok(Vec::new());
        };
        match self.nodes.get(area.root) {
            Some(Node::Column(_)) => Ok(vec![area.root]),
            Some(Node::Split(split)) if split.axis == Axis::X => {
                if split
                    .children
                    .iter()
                    .any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "main horizontal split must contain only Column nodes",
                    });
                }
                Ok(split.children.clone())
            }
            Some(_) => Err(CommandError::InvalidTarget {
                reason: "main dock root must be a Column or Split(X) of Columns",
            }),
            None => Err(CommandError::InvalidTarget {
                reason: "main dock root is missing",
            }),
        }
    }

    pub(super) fn resolve_open_target(
        &self,
        key: &PanelKey,
        metrics: &DockMetrics,
    ) -> Result<DropTarget, CommandError> {
        if let Some(location) = self.last_dock_locations.get(key).copied()
            && let Some(target) = self.target_for_dock_location(location)
            && self.validate_drop_target(target, metrics).is_ok()
        {
            return Ok(target);
        }
        self.default_open_target()
    }

    pub(super) fn target_for_dock_location(
        &self,
        location: LastDockLocation,
    ) -> Option<DropTarget> {
        if let Ok(columns) = self.main_columns(location.side())
            && let Some(column_id) = columns.get(location.column_index()).copied()
            && let Some(Node::Column(column)) = self.nodes.get(column_id)
        {
            if let Some(root_group) = column.groups.get(location.group_index()).copied() {
                if column.mode.is_rail() {
                    let order = self.column_display_order(column_id).ok()?;
                    let index = order
                        .iter()
                        .position(|item| self.is_descendant_or_same(root_group, item.group))
                        .unwrap_or(order.len());
                    return Some(DropTarget::RailSlot {
                        column: column_id,
                        index,
                    });
                }
                if let Some(group) = self.first_tabs_in_subtree(root_group) {
                    let child_count = match self.nodes.get(group) {
                        Some(Node::Tabs(tabs)) => tabs.children.len(),
                        _ => return None,
                    };
                    return Some(DropTarget::Tabs {
                        group,
                        index: child_count,
                    });
                }
            }

            if !column.groups.is_empty() && column.mode.is_rail() {
                let index = self.column_display_order(column_id).ok()?.len();
                return Some(DropTarget::RailSlot {
                    column: column_id,
                    index,
                });
            } else if let Some(last_group) = column.groups.last().copied() {
                return Some(DropTarget::GroupEdge {
                    group: last_group,
                    zone: DropZone::Bottom,
                });
            }
        }

        // Generic-profile trees need not have Column nodes. In that case the
        // closest stable preference is the first tab group under the same side.
        let area = self.main_area(location.side())?;
        let group = self.first_tabs_in_subtree(area.root)?;
        let index = match self.nodes.get(group) {
            Some(Node::Tabs(tabs)) => tabs.children.len(),
            _ => return None,
        };
        Some(DropTarget::Tabs { group, index })
    }

    pub(super) fn default_open_target(&self) -> Result<DropTarget, CommandError> {
        // Prefer the right dock, but use an existing compatible group on the left
        // if the right root has none. If neither side has Tabs, append a right column.
        for side in [DockSide::Right, DockSide::Left] {
            if let Some(area) = self.main_area(side)
                && let Some(group) = self.first_tabs_in_subtree(area.root)
            {
                let index = match self.nodes.get(group) {
                    Some(Node::Tabs(tabs)) => tabs.children.len(),
                    _ => return Err(CommandError::InvalidTarget {
                        reason: "profile default placement did not resolve to a Tabs node",
                    }),
                };
                return Ok(DropTarget::Tabs { group, index });
            }
        }
        let index = self.main_columns(DockSide::Right).map_or(0, |columns| columns.len());
        Ok(DropTarget::MainColumn { side: DockSide::Right, index })
    }

    pub(super) fn first_tabs_in_subtree(&self, root: NodeId) -> Option<NodeId> {
        match self.nodes.get(root)? {
            Node::Tabs(_) => Some(root),
            node => node
                .children()
                .iter()
                .copied()
                .find_map(|child| self.first_tabs_in_subtree(child)),
        }
    }

    pub(super) fn panel_nodes_in_subtree(&self, root: NodeId) -> Vec<NodeId> {
        let mut panels = Vec::new();
        self.collect_panel_nodes(root, &mut panels);
        panels
    }

    pub(super) fn collect_panel_nodes(&self, root: NodeId, panels: &mut Vec<NodeId>) {
        let Some(node) = self.nodes.get(root) else {
            return;
        };
        if matches!(node, Node::Panel(_)) {
            panels.push(root);
            return;
        }
        for child in node.children().iter().copied() {
            self.collect_panel_nodes(child, panels);
        }
    }

    pub(super) fn remember_subtree_dock_locations(&mut self, root: NodeId) {
        for panel_node in self.panel_nodes_in_subtree(root) {
            self.remember_panel_dock_location(panel_node);
        }
    }

    pub(super) fn remember_panel_dock_location(&mut self, panel_node: NodeId) {
        let Some(Node::Panel(panel)) = self.nodes.get(panel_node) else {
            return;
        };
        let Some(instance) = self.panel_instances.get(panel.panel_id) else {
            return;
        };
        let key = instance.key.clone();
        if let Some(location) = self.dock_location_for_node(panel_node) {
            self.last_dock_locations.insert(key, location);
        }
    }

    pub(super) fn dock_location_for_node(&self, node_id: NodeId) -> Option<LastDockLocation> {
        for side in [DockSide::Left, DockSide::Right] {
            let Some(area) = self.main_area(side) else {
                continue;
            };
            if !self.is_descendant_or_same(area.root, node_id) {
                continue;
            }
            if let Ok(columns) = self.main_columns(side) {
                for (column_index, column_id) in columns.iter().copied().enumerate() {
                    let Some(Node::Column(column)) = self.nodes.get(column_id) else {
                        continue;
                    };
                    if let Some(group_index) = column
                        .groups
                        .iter()
                        .position(|group| self.is_descendant_or_same(*group, node_id))
                    {
                        return Some(LastDockLocation::new(side, column_index, group_index));
                    }
                }
            }
            // The Generic profile may use a tree without Column nodes. It still
            // remembers the side; indices zero are the deterministic fallback.
            return Some(LastDockLocation::new(side, 0, 0));
        }
        None
    }

    pub(super) fn find_panel_node(&self, panel_id: PanelId) -> Option<NodeId> {
        self.nodes.iter().find_map(|(id, node)| match node {
            Node::Panel(panel) if panel.panel_id == panel_id => Some(id),
            _ => None,
        })
    }

    pub(super) fn tab_parent_index(&self, node_id: NodeId) -> Option<(NodeId, usize)> {
        let parent = self.find_parent(node_id)?;
        match self.nodes.get(parent) {
            Some(Node::Tabs(tabs)) => tabs
                .children
                .iter()
                .position(|id| *id == node_id)
                .map(|index| (parent, index)),
            _ => None,
        }
    }

    pub(super) fn find_parent(&self, child: NodeId) -> Option<NodeId> {
        self.nodes
            .iter()
            .find_map(|(parent, node)| node.children().contains(&child).then_some(parent))
    }

    /// Shared no-op contract for dropping a group at the edge of an adjacent
    /// sibling under the same Column or vertical Split.
    ///
    /// Callers supply parent IDs so indexed callers can avoid rescanning the
    /// whole arena; both offer generation and drag resolution use this rule.
    pub(crate) fn adjacent_group_edge_is_noop(
        &self,
        source_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        source_parent: Option<NodeId>,
        target_parent: Option<NodeId>,
    ) -> bool {
        if !matches!(zone, DropZone::Top | DropZone::Bottom) {
            return false;
        }
        let (Some(parent), Some(target_parent)) = (source_parent, target_parent) else {
            return false;
        };
        if parent != target_parent {
            return false;
        }
        let siblings = match self.nodes.get(parent) {
            Some(Node::Column(column)) => &column.groups,
            Some(Node::Split(split)) if split.axis == Axis::Y => &split.children,
            _ => return false,
        };
        let (Some(source_index), Some(target_index)) = (
            siblings.iter().position(|id| *id == source_group),
            siblings.iter().position(|id| *id == target_group),
        ) else {
            return false;
        };
        (zone == DropZone::Top && source_index + 1 == target_index)
            || (zone == DropZone::Bottom && target_index + 1 == source_index)
    }

    /// Returns whether a group insertion slot leaves the group in the same
    /// place. The index is measured after removing the source group, matching
    /// the command and drop-zone insertion-slot contract.
    pub(crate) fn group_slot_is_noop(
        &self,
        group: NodeId,
        column_id: NodeId,
        index: usize,
    ) -> bool {
        // Preserve the ownership check: a malformed tree may reference the same
        // group from multiple columns. Do not treat a slot in a non-owner column
        // as a no-op merely because its group list happens to contain that ID.
        if self.find_owner_column(group) != Some(column_id) {
            return false;
        }
        let Some(Node::Column(column)) = self.nodes.get(column_id) else {
            return false;
        };
        let Some(source_index) = column.groups.iter().position(|id| *id == group) else {
            return false;
        };
        index == source_index || index == source_index + 1
    }

    pub(super) fn find_owner_column(&self, group: NodeId) -> Option<NodeId> {
        self.nodes.iter().find_map(|(id, node)| match node {
            Node::Column(column) if column.groups.contains(&group) => Some(id),
            _ => None,
        })
    }

    pub(super) fn rail_column_for_group(&self, group_id: NodeId) -> Option<NodeId> {
        let main_roots = [DockSide::Left, DockSide::Right]
            .into_iter()
            .filter_map(|side| self.main_area(side).map(|area| area.root))
            .collect::<Vec<_>>();
        self.nodes.iter().find_map(|(column_id, node)| match node {
            Node::Column(column)
                if column.mode.is_rail()
                    && main_roots
                        .iter()
                        .any(|root| self.is_descendant_or_same(*root, column_id))
                    && column
                        .groups
                        .iter()
                        .any(|root| self.is_descendant_or_same(*root, group_id)) =>
            {
                Some(column_id)
            }
            _ => None,
        })
    }

    pub(super) fn main_column_position(&self, column_id: NodeId) -> Option<(DockSide, usize)> {
        for side in [DockSide::Left, DockSide::Right] {
            if let Ok(columns) = self.main_columns(side)
                && let Some(index) = columns.iter().position(|id| *id == column_id)
            {
                return Some((side, index));
            }
        }
        None
    }

    pub(super) fn find_floating_root_containing(&self, node_id: NodeId) -> Option<NodeId> {
        self.floating_groups.values().find_map(|group| {
            let root = self.dock_areas.get(group.area)?.root;
            self.is_descendant_or_same(root, node_id).then_some(root)
        })
    }

    pub(super) fn is_descendant_or_same(&self, root: NodeId, target: NodeId) -> bool {
        if root == target {
            return true;
        }
        let Some(node) = self.nodes.get(root) else {
            return false;
        };
        node.children()
            .iter()
            .copied()
            .any(|child| self.is_descendant_or_same(child, target))
    }

    pub(super) fn replace_child(
        &mut self,
        parent: NodeId,
        old_child: NodeId,
        new_child: NodeId,
    ) -> Result<(), CommandError> {
        let Some(parent_node) = self.nodes.get_mut(parent) else {
            return Err(CommandError::MissingNode { node: parent });
        };
        let replaced = match parent_node {
            Node::Split(split) => split
                .children
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Tabs(tabs) => tabs
                .children
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Column(column) => column
                .groups
                .iter_mut()
                .find(|id| **id == old_child)
                .map(|id| *id = new_child)
                .is_some(),
            Node::Panel(_) => false,
        };
        if replaced {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget {
                reason: "target node is not a child of the expected parent",
            })
        }
    }

    pub(super) fn set_area_root(
        &mut self,
        old_root: NodeId,
        new_root: NodeId,
    ) -> Result<(), CommandError> {
        if let Some(area) = self
            .dock_areas
            .values_mut()
            .find(|area| area.root == old_root)
        {
            area.root = new_root;
            return Ok(());
        }
        Err(CommandError::InvalidTarget {
            reason: "target node is not a registered dock-area root",
        })
    }

    pub(super) fn detach_node(&mut self, node_id: NodeId) -> Result<(), CommandError> {
        if !self.nodes.contains_key(node_id) {
            return Err(CommandError::MissingNode { node: node_id });
        }
        if let Some(parent) = self.find_parent(node_id) {
            let Some(parent_node) = self.nodes.get_mut(parent) else {
                return Err(CommandError::MissingNode { node: parent });
            };
            match parent_node {
                Node::Split(split) => {
                    if let Some(index) = split.children.iter().position(|id| *id == node_id) {
                        split.children.remove(index);
                        if index < split.ratios.len() {
                            split.ratios.remove(index);
                        }
                    }
                }
                Node::Tabs(tabs) => {
                    let active_node = tabs.children.get(tabs.active_index).copied();
                    if let Some(index) = tabs.children.iter().position(|id| *id == node_id) {
                        tabs.children.remove(index);
                        if index < tabs.children.len() {
                            if active_node == Some(node_id) {
                                tabs.active_index = index;
                            } else if let Some(active_node) = active_node {
                                tabs.active_index = tabs
                                    .children
                                    .iter()
                                    .position(|id| *id == active_node)
                                    .unwrap_or(index);
                            } else {
                                tabs.active_index =
                                    index.min(tabs.children.len().saturating_sub(1));
                            }
                        } else {
                            tabs.active_index = tabs.children.len().saturating_sub(1);
                        }
                    }
                }
                Node::Column(column) => {
                    if let Some(index) = column.groups.iter().position(|id| *id == node_id) {
                        column.groups.remove(index);
                        if index < column.group_ratios.len() {
                            column.group_ratios.remove(index);
                        }
                    }
                }
                Node::Panel(_) => {
                    return Err(CommandError::InvalidTarget {
                        reason: "Panel leaves cannot own child nodes",
                    });
                }
            }
            return Ok(());
        }

        for index in 0..self.main_areas.len() {
            if let Some(area_id) = self.main_areas[index]
                && self
                    .dock_areas
                    .get(area_id)
                    .is_some_and(|area| area.root == node_id)
            {
                self.main_areas[index] = None;
                self.dock_areas.remove(area_id);
                return Ok(());
            }
        }
        let floating = self.floating_groups.iter().find_map(|(id, group)| {
            self.dock_areas
                .get(group.area)
                .is_some_and(|area| area.root == node_id)
                .then_some((id, group.area))
        });
        if let Some((group_id, area_id)) = floating {
            self.floating_groups.remove(group_id);
            self.dock_areas.remove(area_id);
            return Ok(());
        }
        Err(CommandError::InvalidTarget {
            reason: "node is not attached to a registered dock area or parent",
        })
    }

    pub(super) fn remove_subtree(&mut self, node_id: NodeId) {
        let Some(node) = self.nodes.get(node_id).cloned() else {
            return;
        };
        for child in node.children().iter().copied() {
            self.remove_subtree(child);
        }
        if let Node::Panel(panel) = node {
            self.panel_instances.remove(panel.panel_id);
        }
        self.nodes.remove(node_id);
    }

    pub(super) fn sync_main_column_ratios(
        &mut self,
        column_id: NodeId,
    ) -> Result<(), CommandError> {
        let Some((side, _)) = self.main_column_position(column_id) else {
            return Ok(());
        };
        let columns = self.main_columns(side)?;
        if columns.len() < 2 {
            return Ok(());
        }
        let widths: Vec<f32> = columns
            .iter()
            .map(|id| match self.nodes.get(*id) {
                Some(Node::Column(column)) => Ok(column.width),
                _ => Err(CommandError::InvalidTarget {
                    reason: "main horizontal split contains a non-Column child",
                }),
            })
            .collect::<Result<_, _>>()?;
        let total: f32 = widths.iter().copied().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "total dock column width overflowed",
            });
        }
        let ratios: Vec<f32> = widths.iter().map(|width| *width / total).collect();
        let area = self
            .main_area(side)
            .copied()
            .ok_or(CommandError::InvalidTarget {
                reason: "main dock area disappeared while synchronizing column ratios",
            })?;
        if let Some(Node::Split(split)) = self.nodes.get_mut(area.root) {
            if split.axis != Axis::X {
                return Err(CommandError::InvalidTarget {
                    reason: "main column split must use horizontal axis X",
                });
            }
            split.ratios = ratios;
        }
        Ok(())
    }
}

#[cfg(test)]
mod input_validation_tests {
    use super::*;
    use crate::{Column, ColumnMode, Tabs};

    #[test]
    fn group_slot_noop_requires_the_actual_owner_and_a_valid_adjacent_slot() {
        let mut layout = Layout::new(Profile::Generic);
        let group = layout.nodes.insert(Node::Tabs(Tabs {
            children: Vec::new(),
            active_index: 0,
            collapsed: false,
        }));
        let column_node = || {
            Node::Column(Column {
                width: 280.0,
                mode: ColumnMode::Expanded,
                groups: vec![group],
                group_ratios: vec![1.0],
                last_expanded_width: Some(280.0),
            })
        };
        let owner = layout.nodes.insert(column_node());
        let duplicate_owner = layout.nodes.insert(column_node());

        // Invalid duplicate ownership must not make the second column report
        // a no-op. Valid insertion slots remain no-ops, while an invalid index
        // is left for the command validator to reject.
        assert_eq!(layout.find_owner_column(group), Some(owner));
        assert!(!layout.group_slot_is_noop(group, duplicate_owner, 0));
        assert!(layout.group_slot_is_noop(group, owner, 0));
        assert!(layout.group_slot_is_noop(group, owner, 1));
        assert!(!layout.group_slot_is_noop(group, owner, 2));
    }
}
