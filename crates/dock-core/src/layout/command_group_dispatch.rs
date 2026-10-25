use super::*;
use crate::AllowedArea;

impl Layout {
    pub(super) fn apply_group_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MoveGroup {
                group,
                column,
                index,
            } => {
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
                if source_parent == Some(*column) {
                    let source_index = target_groups.iter().position(|id| id == group);
                    if let Some(source_index) = source_index {
                        if *index == source_index || *index == source_index + 1 {
                            return Ok(CommandApplication::persistent());
                        }
                        let adjusted_index = if *index > source_index {
                            *index - 1
                        } else {
                            *index
                        };
                        let inverse_index = if source_index > adjusted_index {
                            source_index + 1
                        } else {
                            source_index
                        };
                        let inverse = Some(DockCommand::MoveGroup {
                            group: *group,
                            column: *column,
                            index: inverse_index,
                        });
                        let Some(Node::Column(target_column)) = self.nodes.get_mut(*column) else {
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
                        self.remember_subtree_dock_locations(*group);
                        return Ok(CommandApplication {
                            actual_value: None,
                            inverse,
                            persistent: true,
                        });
                    }
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
                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                let new_len = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.groups.len(),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "MoveGroup target changed unexpectedly",
                        });
                    }
                };
                self.insert_group_at_column(*column, *group, (*index).min(new_len), None)?;
                self.remember_subtree_dock_locations(*group);
                Ok(CommandApplication {
                    actual_value: None,
                    inverse,
                    persistent: true,
                })
            }
            DockCommand::MoveGroupToMainColumn { group, side, index } => {
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

                let source_column = self.find_owner_column(*group);
                let source_position =
                    source_column.and_then(|column| self.main_column_position(column));
                let source_group_index =
                    source_column.and_then(|column| match self.nodes.get(column) {
                        Some(Node::Column(value)) => value.groups.iter().position(|id| id == group),
                        _ => None,
                    });
                let source_is_sole_group = source_column.is_some_and(|column| {
                    matches!(self.nodes.get(column), Some(Node::Column(value))
                        if value.groups.len() == 1 && value.groups[0] == *group)
                });

                // If this group already owns a single-group main column on the
                // destination side, move/reorder that column itself. An insertion
                // slot immediately before or after it represents its current place.
                if source_is_sole_group
                    && let Some((source_side, source_index)) = source_position
                    && source_side == *side
                {
                    if *index == source_index || *index == source_index + 1 {
                        return Ok(CommandApplication::persistent());
                    }

                    let area_root = self.main_area(*side).map(|area| area.root).ok_or(
                        CommandError::InvalidTarget {
                            reason: "destination main dock area disappeared",
                        },
                    )?;
                    let Some(Node::Split(mut split)) = self.nodes.get(area_root).cloned() else {
                        // A single-column dock has only its current slot;
                        // a valid out-of-range insertion was rejected above.
                        return Ok(CommandApplication::persistent());
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
                    let adjusted_index = if *index > source_index {
                        *index - 1
                    } else {
                        *index
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
                    self.remember_subtree_dock_locations(*group);
                    let inverse_index = if source_index > adjusted_index {
                        source_index + 1
                    } else {
                        source_index
                    };

                    return Ok(CommandApplication {
                        actual_value: None,
                        inverse: Some(DockCommand::MoveGroupToMainColumn {
                            group: *group,
                            side: *side,
                            index: inverse_index,
                        }),
                        persistent: true,
                    });
                }

                let inverse = match (source_column, source_group_index, source_position) {
                    (Some(column), Some(group_index), _)
                        if matches!(
                            self.nodes.get(column),
                            Some(Node::Column(value)) if value.groups.len() > 1
                        ) =>
                    {
                        Some(DockCommand::MoveGroup {
                            group: *group,
                            column,
                            index: group_index,
                        })
                    }
                    (_, _, Some((source_side, source_index))) => {
                        Some(DockCommand::MoveGroupToMainColumn {
                            group: *group,
                            side: source_side,
                            index: source_index,
                        })
                    }
                    _ => None,
                };

                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                let new_column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![*group],
                    group_ratios: vec![1.0],
                    last_expanded_width: Some(metrics.default_column_width),
                }));
                self.insert_existing_column_at(*side, *index, new_column, None, metrics)?;
                self.remember_subtree_dock_locations(*group);
                Ok(CommandApplication {
                    actual_value: None,
                    inverse,
                    persistent: true,
                })
            }
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

                    // Dropping a whole group into the center of a collapsed
                    // target also reveals that target in the Photoshop profile.
                    // This is inside the transaction candidate, so an invalid
                    // merge still leaves the live Layout unchanged.
                    if self.profile == crate::Profile::Photoshop
                        && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(*target_group)
                    {
                        tabs.collapsed = false;
                    }
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
}
