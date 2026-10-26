use super::*;

impl FloatingZoneContext<'_> {
    pub(super) fn append_center_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // Multiple groups can share a vertical position in a horizontal floating
        // composition, so center zones are emitted per group.
        for target_group in target_groups {
            let tabs = match self.layout.node(target_group.group) {
                Some(Node::Tabs(tabs)) if !tabs.collapsed => tabs,
                _ => continue,
            };
            let is_self = matches!(self.source, DropSource::Group { .. })
                && contains_node(self.tree_index, self.source_root, target_group.group);
            let panel_already_in_group = match self.source {
                DropSource::Panel { panel } => {
                    panel_node(self.tree_index, panel)
                        .and_then(|node_id| direct_parent(self.tree_index, node_id))
                        == Some(target_group.group)
                }
                DropSource::Group { .. } => false,
            };
            if is_self || panel_already_in_group {
                continue;
            }

            let target = match self.source {
                DropSource::Panel { .. } => DropTarget::Tabs {
                    group: target_group.group,
                    index: tabs.children.len(),
                },
                DropSource::Group { .. } => DropTarget::GroupTabs {
                    group: target_group.group,
                    index: tabs.children.len(),
                },
            };
            push_zone(
                self.zones,
                target,
                DropZone::FloatingCenter,
                group_center_rect(target_group.rect, self.metrics)?,
            )?;
        }
        Ok(())
    }
}
