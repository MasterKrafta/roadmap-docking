use super::*;

impl FloatingZoneContext<'_> {
    pub(super) fn append_side_zones(
        &mut self,
        target_groups: &[GroupRect],
    ) -> Result<(), ComputeRectsError> {
        // The Generic profile supports horizontal nested splits within a
        // floating area; Photoshop uses its column-edge policy instead.
        for target_group in target_groups {
            if matches!(self.source, DropSource::Group { .. })
                && contains_node(self.tree_index, self.source_root, target_group.group)
            {
                continue;
            }
            for (edge, kind) in [
                (DropZone::Left, DropZone::Left),
                (DropZone::Right, DropZone::Right),
            ] {
                if group_edge_is_noop(
                    self.layout,
                    self.tree_index,
                    self.source,
                    target_group.group,
                    edge,
                ) {
                    continue;
                }
                push_zone(
                    self.zones,
                    DropTarget::GroupEdge {
                        group: target_group.group,
                        zone: edge,
                    },
                    kind,
                    group_edge_rect(target_group.rect, edge, self.metrics.edge_drop_width)?,
                )?;
            }
        }
        Ok(())
    }
}
