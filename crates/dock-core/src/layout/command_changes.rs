use super::*;

impl Layout {


    pub(super) fn same_state(&self, other: &Self) -> bool {
        self.same_persistent_state(other)
            && self.flyout == other.flyout
            && self.flyout_auto_collapse == other.flyout_auto_collapse
            && self.flyout_widths == other.flyout_widths
            && self.flyout_width_order == other.flyout_width_order
    }

    pub(super) fn changed_node_ids(&self, other: &Self, command: &DockCommand) -> Vec<NodeId> {
        let mut changed = Vec::new();
        for (id, node) in self.nodes.iter() {
            if other.nodes.get(id) != Some(node) {
                changed.push(id);
            }
        }
        for (id, _) in other.nodes.iter() {
            if !self.nodes.contains_key(id) {
                changed.push(id);
            }
        }

        // Some persistent presentation commands change geometry/stacking without
        // changing the underlying Node values; return roots for adapter invalidation.
        let mut affected = Vec::new();
        match command {
            DockCommand::MoveFloating { group, .. }
            | DockCommand::ResizeFloatingBounds { group, .. }
            | DockCommand::ResizeFloating { group, .. }
            | DockCommand::RaiseFloating { group }
            | DockCommand::DockFloating { group, .. } => {
                for layout in [self, other] {
                    if let Some(floating) = layout.floating_group(*group)
                        && let Some(area) = layout.dock_area(floating.area)
                    {
                        affected.push(area.root);
                    }
                }
            }
            DockCommand::SetChrome { .. } => {
                for layout in [self, other] {
                    affected.extend(layout.main_areas.iter().flatten().filter_map(|area_id| layout.dock_area(*area_id).map(|area| area.root)));
                    affected.extend(layout.floating_groups.values().filter_map(|group| layout.dock_area(group.area).map(|area| area.root)));
                }
            }
            _ => {}
        }
        for node in affected {
            if !changed.contains(&node) {
                changed.push(node);
            }
        }
        changed
    }
}
