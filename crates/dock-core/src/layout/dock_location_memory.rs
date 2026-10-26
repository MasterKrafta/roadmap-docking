use super::*;

impl Layout {
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
                self.last_dock_locations.insert(key.clone(), location);
                self.last_dock_location_order.retain(|remembered| *remembered != key);
                self.last_dock_location_order.push_back(key);
                while self.last_dock_location_order.len() > MAX_REMEMBERED_DOCK_LOCATIONS {
                    if let Some(expired) = self.last_dock_location_order.pop_front() {
                        self.last_dock_locations.remove(&expired);
                    }
                }
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
}
