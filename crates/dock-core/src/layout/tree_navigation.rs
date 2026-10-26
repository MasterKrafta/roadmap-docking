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

    pub(super) fn panel_ids_in_subtree(&self, root: NodeId) -> Vec<PanelId> {
        self.panel_nodes_in_subtree(root)
            .into_iter()
            .filter_map(|node_id| match self.nodes.get(node_id) {
                Some(Node::Panel(panel)) => Some(panel.panel_id),
                _ => None,
            })
            .collect()
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
}
