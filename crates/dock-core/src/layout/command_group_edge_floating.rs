use super::*;
use super::helpers::split_sibling_ratios;

impl Layout {
    pub(super) fn insert_group_edge_in_floating_stack(
        &mut self,
        new_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        split_ratio: f32,
    ) -> Result<(), CommandError> {
        let floating_root = self
            .find_floating_root_containing(target_group)
            .ok_or(CommandError::InvalidTarget {
                reason: "target group is not in a Photoshop column or floating stack",
            })?;
        match zone {
            DropZone::Left | DropZone::Right => Err(CommandError::InvalidTarget {
                reason: "horizontal splits are not supported inside a floating stack",
            }),
            DropZone::Top | DropZone::Bottom => {
                if floating_root == target_group {
                    let (children, ratios) = if zone == DropZone::Top {
                        (
                            vec![new_group, target_group],
                            vec![split_ratio, 1.0 - split_ratio],
                        )
                    } else {
                        (
                            vec![target_group, new_group],
                            vec![1.0 - split_ratio, split_ratio],
                        )
                    };
                    let split = self.nodes.insert(Node::Split(crate::Split {
                        axis: Axis::Y,
                        children,
                        ratios,
                    }));
                    self.set_area_root(target_group, split)?;
                    Ok(())
                } else {
                    let parent = self.find_parent(target_group).ok_or(
                        CommandError::InvalidTarget {
                            reason: "floating group parent was not found",
                        },
                    )?;
                    match self.nodes.get(parent) {
                        Some(Node::Split(split))
                            if split.axis == Axis::Y
                                && split.children.contains(&target_group) =>
                        {
                            let target_index = split
                                .children
                                .iter()
                                .position(|id| *id == target_group)
                                .ok_or(CommandError::InvalidTarget {
                                    reason: "target group is not present in the floating stack",
                                })?;
                            let insert_at = if zone == DropZone::Top {
                                target_index
                            } else {
                                target_index + 1
                            };
                            let old_len = split.children.len();
                            let new_ratios = split_sibling_ratios(
                                &split.ratios,
                                old_len,
                                target_index,
                                insert_at,
                                split_ratio,
                            );
                            let Some(Node::Split(split)) = self.nodes.get_mut(parent) else {
                                return Err(CommandError::InvalidTarget {
                                    reason: "floating stack changed unexpectedly",
                                });
                            };
                            split.children.insert(insert_at, new_group);
                            split.ratios = new_ratios;
                            Ok(())
                        }
                        _ => Err(CommandError::InvalidTarget {
                            reason: "floating stack must be a vertical Split of Tabs",
                        }),
                    }
                }
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group-edge target requires a cardinal edge zone",
            }),
        }
    }
}
