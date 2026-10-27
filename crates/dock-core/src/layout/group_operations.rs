//! Group command dispatch, transfer, merging, and edge insertion.

use super::*;
use super::helpers::split_sibling_ratios;
use crate::AllowedArea;

impl Layout {

    pub(super) fn apply_group_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MergeGroup {
                group,
                target_group,
                index,
            } => {
                let panel_nodes = self.panel_nodes_in_subtree(*group);
                if group != target_group {
                    let target = DropTarget::GroupTabs {
                        group: *target_group,
                        index: *index,
                    };
                    self.validate_drop_target(target, metrics)?;
                    let moving_panels = self.panel_ids_in_subtree(*group);
                    self.validate_panel_ids_allowed_for_target(&moving_panels, target)?;

                    self.reveal_merge_target_for_profile(*target_group);
                }
                self.merge_group_into_tabs(*group, *target_group, *index)?;
                if group == target_group {
                    return Ok(CommandApplication::persistent());
                }
                for panel_node in panel_nodes {
                    self.remember_panel_dock_location(panel_node);
                }
                Ok(CommandApplication::persistent())
            }
            DockCommand::SplitGroup {
                group,
                target_group,
                zone,
                ratio,
            } => {
                if !ratio.is_finite() || *ratio <= 0.0 || *ratio >= 1.0 {
                    return Err(CommandError::InvalidValue {
                        field: "ratio",
                        reason: "split ratio must be finite and strictly between 0 and 1",
                    });
                }
                if !matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
                    return Err(if self.nodes.contains_key(*group) {
                        CommandError::InvalidTarget {
                            reason: "SplitGroup source must be a Tabs node",
                        }
                    } else {
                        CommandError::MissingNode { node: *group }
                    });
                }
                if !matches!(self.nodes.get(*target_group), Some(Node::Tabs(_))) {
                    return Err(if self.nodes.contains_key(*target_group) {
                        CommandError::InvalidTarget {
                            reason: "SplitGroup target must be a Tabs node",
                        }
                    } else {
                        CommandError::MissingNode {
                            node: *target_group,
                        }
                    });
                }
                let target = DropTarget::GroupEdge {
                    group: *target_group,
                    zone: *zone,
                };
                self.validate_drop_target(target, metrics)?;
                if group != target_group {
                    let moving_panels = self.panel_ids_in_subtree(*group);
                    self.validate_panel_ids_allowed_for_target(&moving_panels, target)?;
                }
                if group == target_group {
                    return Ok(CommandApplication::persistent());
                }

                // A horizontal split that reproduces the existing neighboring
                // column placement is a no-op when the moving group is the
                // source column's only group. Do not replace the Column node or
                // perturb ratios for a drop that leaves the tree unchanged.
                if matches!(zone, DropZone::Left | DropZone::Right)
                    && let (Some(source_column), Some(target_column)) = (
                        self.find_owner_column(*group),
                        self.find_owner_column(*target_group),
                    )
                {
                    let source_is_single_group_column = matches!(
                        self.nodes.get(source_column),
                        Some(Node::Column(column))
                            if column.groups.len() == 1 && column.groups[0] == *group
                    );
                    if source_is_single_group_column
                        && let (
                            Some((source_side, source_index)),
                            Some((target_side, target_index)),
                        ) = (
                            self.main_column_position(source_column),
                            self.main_column_position(target_column),
                        )
                    {
                        let already_at_requested_edge = source_side == target_side
                            && ((*zone == DropZone::Left && source_index + 1 == target_index)
                                || (*zone == DropZone::Right && target_index + 1 == source_index));
                        if already_at_requested_edge {
                            return Ok(CommandApplication::persistent());
                        }
                    }
                }

                let source_parent = self.find_parent(*group);
                let target_parent = self.find_parent(*target_group);
                if self.adjacent_group_edge_is_noop(
                    *group,
                    *target_group,
                    *zone,
                    source_parent,
                    target_parent,
                ) {
                    return Ok(CommandApplication::persistent());
                }

                let source_column = self.find_owner_column(*group);
                let source_index = source_column.and_then(|column| match self.nodes.get(column) {
                    Some(Node::Column(column_data)) if column_data.groups.len() > 1 => column_data
                        .groups
                        .iter()
                        .position(|id| id == group)
                        .map(|index| (column, index)),
                    _ => None,
                });
                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                self.insert_group_edge(*group, *target_group, *zone, Some(*ratio), metrics)?;
                self.remember_subtree_dock_locations(*group);
                let inverse = source_index.and_then(|(column, original_index)| {
                    let restore_index = if self.find_owner_column(*group) == Some(column) {
                        match self.nodes.get(column) {
                            Some(Node::Column(column_data)) => {
                                column_data.groups.iter().position(|id| id == group).map(
                                    |current_index| {
                                        if original_index > current_index {
                                            original_index + 1
                                        } else {
                                            original_index
                                        }
                                    },
                                )
                            }
                            _ => None,
                        }
                    } else {
                        Some(original_index)
                    };
                    restore_index.map(|index| DockCommand::MoveGroup {
                        group: *group,
                        column,
                        index,
                    })
                });
                Ok(CommandApplication {
                    actual_value: None,
                    inverse,
                    persistent: true,
                })
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "apply group command received an unsupported command",
            }),
        }
    }

    /// Photoshop profile treats merging into a collapsed group as revealing it.
    /// Called only after merge-target and panel constraints have been validated.
    fn reveal_merge_target_for_profile(&mut self, target_group: NodeId) {
        if self.profile == crate::Profile::Photoshop
            && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(target_group)
        {
            tabs.collapsed = false;
        }
    }

    pub(super) fn merge_group_into_tabs(
        &mut self,
        group: NodeId,
        target_group: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        if group == target_group {
            return match self.nodes.get(group) {
                Some(Node::Tabs(_)) => Ok(()),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "MergeGroup source must be a Tabs node",
                }),
                None => Err(CommandError::MissingNode { node: group }),
            };
        }
        let (source_children, source_active) = match self.nodes.get(group) {
            Some(Node::Tabs(tabs)) if !tabs.children.is_empty() => {
                (tabs.children.clone(), tabs.active_index)
            }
            Some(Node::Tabs(_)) => {
                return Err(CommandError::InvalidTarget {
                    reason: "cannot merge an empty source group",
                });
            }
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "MergeGroup source must be a Tabs node",
                });
            }
            None => return Err(CommandError::MissingNode { node: group }),
        };
        let target_len = match self.nodes.get(target_group) {
            Some(Node::Tabs(tabs)) => tabs.children.len(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "MergeGroup target must be a Tabs node",
                });
            }
            None => return Err(CommandError::MissingNode { node: target_group }),
        };
        if index > target_len {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "group-tab insertion index is outside the target group",
            });
        }
        if self.is_descendant_or_same(group, target_group)
            || self.is_descendant_or_same(target_group, group)
        {
            return Err(CommandError::InvalidTarget {
                reason: "cannot merge ancestor or descendant Tabs groups",
            });
        }

        self.detach_node(group)?;
        if let Some(Node::Tabs(source)) = self.nodes.get_mut(group) {
            source.children.clear();
            source.active_index = 0;
        }
        self.nodes.remove(group);

        let Some(Node::Tabs(target)) = self.nodes.get_mut(target_group) else {
            return Err(CommandError::InvalidTarget {
                reason: "MergeGroup target changed unexpectedly",
            });
        };
        for (offset, child) in source_children.iter().copied().enumerate() {
            target.children.insert(index + offset, child);
        }
        target.active_index = index + source_active.min(source_children.len() - 1);
        Ok(())
    }

    pub(super) fn insert_group_edge(
        &mut self,
        new_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        ratio: Option<f32>,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if new_group == target_group {
            return Ok(());
        }
        if !matches!(self.nodes.get(new_group), Some(Node::Tabs(_)))
            || !matches!(self.nodes.get(target_group), Some(Node::Tabs(_)))
        {
            return Err(CommandError::InvalidTarget {
                reason: "group-edge operations require Tabs nodes",
            });
        }
        let split_ratio = ratio.unwrap_or(0.5);
        if !split_ratio.is_finite() || split_ratio <= 0.0 || split_ratio >= 1.0 {
            return Err(CommandError::InvalidValue {
                field: "ratio",
                reason: "split ratio must be between zero and one",
            });
        }

        if self.profile == Profile::Photoshop {
            if let Some(column_id) = self.find_owner_column(target_group) {
                self.insert_group_edge_in_photoshop_column(
                    new_group,
                    target_group,
                    zone,
                    split_ratio,
                    column_id,
                    metrics,
                )
            } else {
                self.insert_group_edge_in_floating_stack(
                    new_group,
                    target_group,
                    zone,
                    split_ratio,
                )
            }
        } else {
            self.insert_group_edge_generic(
                new_group,
                target_group,
                zone,
                split_ratio,
            )
        }
    }

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

    pub(super) fn insert_group_edge_in_photoshop_column(
        &mut self,
        new_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        split_ratio: f32,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        match zone {
            DropZone::Top | DropZone::Bottom => {
                let groups = match self.nodes.get(column_id) {
                    Some(Node::Column(column)) => column.groups.clone(),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "target group has no owning column",
                        });
                    }
                };
                let target_index = groups.iter().position(|id| *id == target_group).ok_or(
                    CommandError::InvalidTarget {
                        reason: "target group is not in its owning column",
                    },
                )?;
                let index = if zone == DropZone::Top {
                    target_index
                } else {
                    target_index + 1
                };
                self.insert_group_at_column(
                    column_id,
                    new_group,
                    index,
                    Some((split_ratio, target_index)),
                )
            }
            DropZone::Left | DropZone::Right => {
                let (side, column_index) = self.main_column_position(column_id).ok_or(
                    CommandError::InvalidTarget {
                        reason: "horizontal group splits require a column in a main dock",
                    },
                )?;
                let insert_at = if zone == DropZone::Left {
                    column_index
                } else {
                    column_index + 1
                };
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![new_group],
                    group_ratios: vec![1.0],
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(
                    side,
                    insert_at,
                    column,
                    Some((split_ratio, column_index)),
                    metrics,
                )
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group-edge target requires a cardinal edge zone",
            }),
        }
    }
}

pub(super) struct GroupMainColumnSource {
    pub(super) column: Option<NodeId>,
    pub(super) position: Option<(DockSide, usize)>,
    pub(super) group_index: Option<usize>,
    pub(super) is_sole_group: bool,
}

impl Layout {
    pub(super) fn apply_group_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MoveGroup { group, column, index } => {
                self.apply_move_group_command(group, column, index)
            }
            DockCommand::MoveGroupToMainColumn { group, side, index } => {
                self.apply_move_group_to_main_column_command(group, side, index, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "group transfer dispatcher received an unsupported command",
            }),
        }
    }

    pub(super) fn apply_move_group_command(
        &mut self,
        group: &NodeId,
        column: &NodeId,
        index: &usize,
    ) -> Result<CommandApplication, CommandError> {
        match self.nodes.get(*group) {
            Some(Node::Tabs(_)) => {}
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "MoveGroup source must be a Tabs node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *group }),
        };
        let target_groups = match self.nodes.get(*column) {
            Some(Node::Column(column)) => column.groups.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "MoveGroup target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *column }),
        };
        if *index > target_groups.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "group insertion index is outside the column",
            });
        }
        let target_area = self.allowed_area_for_column_target(*column)?;
        let moving_panels = self.panel_ids_in_subtree(*group);
        self.validate_panel_ids_allowed_for_area(&moving_panels, target_area)?;
        let source_parent = self.find_parent(*group);
        if source_parent == Some(*column)
            && let Some(application) = self.try_reorder_group_within_column(
                *group,
                *column,
                *index,
                &target_groups,
            )?
        {
            return Ok(application);
        }
        let source_column = source_parent.and_then(|parent| match self.nodes.get(parent) {
            Some(Node::Column(column)) if column.groups.len() > 1 => column
                .groups
                .iter()
                .position(|id| *id == *group)
                .map(|index| (parent, index)),
            _ => None,
        });
        let inverse =
            source_column.map(|(source_column, source_index)| DockCommand::MoveGroup {
                group: *group,
                column: source_column,
                index: source_index,
            });
        self.move_group_to_existing_column(*group, *column, *index)?;
        Ok(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        })
    }

    pub(super) fn apply_move_group_to_main_column_command(
        &mut self,
        group: &NodeId,
        side: &DockSide,
        index: &usize,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
            return Err(if self.nodes.contains_key(*group) {
                CommandError::InvalidTarget {
                    reason: "MoveGroupToMainColumn source must be a Tabs node",
                }
            } else {
                CommandError::MissingNode { node: *group }
            });
        }

        let target_columns = self.main_columns(*side)?;
        if *index > target_columns.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "main-column insertion index is outside the dock",
            });
        }

        let moving_panels = self.panel_ids_in_subtree(*group);
        self.validate_panel_ids_allowed_for_area(&moving_panels, AllowedArea::MAIN_DOCK)?;

        let source = self.group_main_column_source(*group);

        if let Some(application) = self.try_reorder_sole_group_main_column(
            *group,
            *side,
            *index,
            source.column,
            source.position,
            source.is_sole_group,
        )? {
            return Ok(application);
        }

        let inverse = self.inverse_for_move_group_to_main_column(*group, &source);

        self.move_group_to_new_main_column(*group, *side, *index, metrics)?;
        Ok(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        })
    }

    pub(super) fn move_group_to_existing_column(
        &mut self,
        group: NodeId,
        column: NodeId,
        index: usize,
    ) -> Result<(), CommandError> {
        self.remember_subtree_dock_locations(group);
        self.detach_node(group)?;
        let new_len = match self.nodes.get(column) {
            Some(Node::Column(value)) => value.groups.len(),
            _ => {
                return Err(CommandError::InvalidTarget {
                    reason: "MoveGroup target changed unexpectedly",
                });
            }
        };
        self.insert_group_at_column(column, group, index.min(new_len), None)?;
        self.remember_subtree_dock_locations(group);
        Ok(())
    }

    pub(super) fn move_group_to_new_main_column(
        &mut self,
        group: NodeId,
        side: DockSide,
        index: usize,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        self.remember_subtree_dock_locations(group);
        self.detach_node(group)?;
        let new_column = self.nodes.insert(Node::Column(crate::Column {
            width: metrics.default_column_width,
            mode: ColumnMode::Expanded,
            groups: vec![group],
            group_ratios: vec![1.0],
            last_expanded_width: Some(metrics.default_column_width),
        }));
        self.insert_existing_column_at(side, index, new_column, None, metrics)?;
        self.remember_subtree_dock_locations(group);
        Ok(())
    }

    pub(super) fn group_main_column_source(&self, group: NodeId) -> GroupMainColumnSource {
        let column = self.find_owner_column(group);
        let position = column.and_then(|column| self.main_column_position(column));
        let group_index = column.and_then(|column| match self.nodes.get(column) {
            Some(Node::Column(value)) => value.groups.iter().position(|id| *id == group),
            _ => None,
        });
        let is_sole_group = column.is_some_and(|column| {
            matches!(self.nodes.get(column), Some(Node::Column(value))
                if value.groups.len() == 1 && value.groups[0] == group)
        });

        GroupMainColumnSource {
            column,
            position,
            group_index,
            is_sole_group,
        }
    }

    pub(super) fn inverse_for_move_group_to_main_column(
        &self,
        group: NodeId,
        source: &GroupMainColumnSource,
    ) -> Option<DockCommand> {
        match (source.column, source.group_index, source.position) {
            (Some(column), Some(group_index), _)
                if matches!(
                    self.nodes.get(column),
                    Some(Node::Column(value)) if value.groups.len() > 1
                ) =>
            {
                Some(DockCommand::MoveGroup {
                    group,
                    column,
                    index: group_index,
                })
            }
            (_, _, Some((source_side, source_index))) => {
                Some(DockCommand::MoveGroupToMainColumn {
                    group,
                    side: source_side,
                    index: source_index,
                })
            }
            _ => None,
        }
    }

    pub(super) fn try_reorder_group_within_column(
        &mut self,
        group: NodeId,
        column: NodeId,
        index: usize,
        target_groups: &[NodeId],
    ) -> Result<Option<CommandApplication>, CommandError> {
        let Some(source_index) = target_groups.iter().position(|id| *id == group) else {
            return Ok(None);
        };
        if index == source_index || index == source_index + 1 {
            return Ok(Some(CommandApplication::persistent()));
        }

        let adjusted_index = if index > source_index {
            index - 1
        } else {
            index
        };
        let inverse_index = if source_index > adjusted_index {
            source_index + 1
        } else {
            source_index
        };
        let inverse = Some(DockCommand::MoveGroup {
            group,
            column,
            index: inverse_index,
        });
        let Some(Node::Column(target_column)) = self.nodes.get_mut(column) else {
            return Err(CommandError::InvalidTarget {
                reason: "MoveGroup target changed unexpectedly",
            });
        };
        let moved_group = target_column.groups.remove(source_index);
        let moved_ratio = target_column.group_ratios.remove(source_index);
        target_column.groups.insert(adjusted_index, moved_group);
        target_column
            .group_ratios
            .insert(adjusted_index, moved_ratio);
        self.remember_subtree_dock_locations(group);

        Ok(Some(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        }))
    }

    pub(super) fn try_reorder_sole_group_main_column(
        &mut self,
        group: NodeId,
        side: DockSide,
        index: usize,
        source_column: Option<NodeId>,
        source_position: Option<(DockSide, usize)>,
        source_is_sole_group: bool,
    ) -> Result<Option<CommandApplication>, CommandError> {
        let Some((source_side, source_index)) = source_position else {
            return Ok(None);
        };
        if !source_is_sole_group || source_side != side {
            return Ok(None);
        }

        // An insertion slot immediately before or after the column is its current place.
        if index == source_index || index == source_index + 1 {
            return Ok(Some(CommandApplication::persistent()));
        }

        let area_root = self.main_area(side).map(|area| area.root).ok_or(
            CommandError::InvalidTarget {
                reason: "destination main dock area disappeared",
            },
        )?;
        let Some(Node::Split(mut split)) = self.nodes.get(area_root).cloned() else {
            // A single-column dock has only its current slot; a valid out-of-range
            // insertion was rejected before attempting this reorder.
            return Ok(Some(CommandApplication::persistent()));
        };
        if split.axis != Axis::X
            || source_column.is_none_or(|column| !split.children.contains(&column))
        {
            return Err(CommandError::InvalidTarget {
                reason: "main dock column order must be represented by Split(X)",
            });
        }
        let Some(ratio_index) = split
            .children
            .iter()
            .position(|id| Some(*id) == source_column)
        else {
            return Err(CommandError::InvalidTarget {
                reason: "source column is absent from the main dock split",
            });
        };
        if ratio_index >= split.ratios.len() {
            return Err(CommandError::InvalidTarget {
                reason: "main dock column ratio is missing",
            });
        }

        let moved_column = split.children.remove(ratio_index);
        let moved_ratio = split.ratios.remove(ratio_index);
        let adjusted_index = if index > source_index {
            index - 1
        } else {
            index
        };
        if adjusted_index > split.children.len() {
            return Err(CommandError::InvalidValue {
                field: "index",
                reason: "adjusted main-column insertion index is outside the dock",
            });
        }
        split.children.insert(adjusted_index, moved_column);
        split.ratios.insert(adjusted_index, moved_ratio);
        if let Some(Node::Split(current)) = self.nodes.get_mut(area_root) {
            *current = split;
        }
        self.remember_subtree_dock_locations(group);
        let inverse_index = if source_index > adjusted_index {
            source_index + 1
        } else {
            source_index
        };

        Ok(Some(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::MoveGroupToMainColumn {
                group,
                side,
                index: inverse_index,
            }),
            persistent: true,
        }))
    }
}
