use super::*;

impl Layout {
    pub(super) fn insert_group_edge_generic(
        &mut self,
        new_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        split_ratio: f32,
    ) -> Result<(), CommandError> {
        let parent = self.find_parent(target_group);
        let axis = match zone {
            DropZone::Left | DropZone::Right => Axis::X,
            DropZone::Top | DropZone::Bottom => Axis::Y,
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "group-edge target requires a cardinal edge zone",
                });
            }
        };
        let children = match zone {
            DropZone::Left | DropZone::Top => vec![new_group, target_group],
            DropZone::Right | DropZone::Bottom => vec![target_group, new_group],
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "group-edge target requires a cardinal edge zone",
                });
            }
        };
        let ratios = match zone {
            DropZone::Left | DropZone::Top => vec![split_ratio, 1.0 - split_ratio],
            DropZone::Right | DropZone::Bottom => vec![1.0 - split_ratio, split_ratio],
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "group-edge target requires a cardinal edge zone",
                });
            }
        };
        let split = self.nodes.insert(Node::Split(crate::Split {
            axis,
            children,
            ratios,
        }));
        if let Some(parent) = parent {
            self.replace_child(parent, target_group, split)?;
        } else {
            self.set_area_root(target_group, split)?;
        }
        Ok(())
    }
}
