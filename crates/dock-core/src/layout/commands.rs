use super::helpers::*;
use super::*;
use crate::AllowedArea;

impl Layout {
    /// Applies one command transactionally. All edits happen on a candidate copy;
    /// the live layout is replaced only after normalization and validation succeed.
    pub fn apply(
        &mut self,
        command: DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandOutcome, CommandError> {
        tracing::debug!(?command, version = self.version, "applying layout command");
        let prior_version = self.version;
        let result = self.apply_transaction(&command, metrics);
        match &result {
            Ok(outcome) if outcome.new_version != prior_version => {
                tracing::info!(
                    ?command,
                    old_version = prior_version,
                    new_version = outcome.new_version,
                    "layout command committed"
                );
            }
            Ok(_) => {
                tracing::debug!(
                    ?command,
                    version = prior_version,
                    "layout command accepted without version change"
                );
            }
            Err(error) => {
                tracing::warn!(
                    ?command,
                    ?error,
                    version = prior_version,
                    "layout command rejected"
                );
            }
        }
        result
    }

    pub(super) fn apply_transaction(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandOutcome, CommandError> {
        self.validate_cached(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;

        let mut candidate = self.clone();
        let applied = candidate.apply_command(command, metrics)?;
        candidate
            .normalize_in_place(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;
        candidate.validation_cache.set(Some((candidate.version, *metrics)));

        if candidate.same_state(self) {
            return Ok(CommandOutcome {
                changed_nodes: Vec::new(),
                new_version: self.version,
                actual_value: applied.actual_value,
                inverse: None,
            });
        }

        if !applied.persistent {
            candidate.version = self.version;
            *self = candidate;
            return Ok(CommandOutcome {
                changed_nodes: Vec::new(),
                new_version: self.version,
                actual_value: applied.actual_value,
                inverse: None,
            });
        }

        let next_version = self
            .version
            .checked_add(1)
            .ok_or(CommandError::InvalidValue {
                field: "version",
                reason: "layout version overflow",
            })?;
        let changed_nodes = self.changed_node_ids(&candidate, command);
        candidate.version = next_version;
        candidate.validation_cache.set(Some((next_version, *metrics)));
        *self = candidate;

        Ok(CommandOutcome {
            changed_nodes,
            new_version: next_version,
            actual_value: applied.actual_value,
            inverse: applied.inverse,
        })
    }

    pub(super) fn apply_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::OpenPanel {
                key,
                constraints,
                target,
            } => {
                constraints
                    .validate()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "constraints",
                        reason: geometry_error_reason(error),
                    })?;

                // Re-opening an existing singleton activates it instead of toggling
                // it closed. If it lives in a rail, the Window menu also reveals its flyout.
                if let Some((existing_id, _)) = self.panel_instances.iter().find(|(_, instance)| {
                    instance.key == *key
                        && (constraints.singleton || instance.constraints.singleton)
                }) {
                    let panel_node = self
                        .find_panel_node(existing_id)
                        .ok_or(CommandError::MissingPanel { panel: existing_id })?;
                    let Some((group, index)) = self.tab_parent_index(panel_node) else {
                        return Ok(CommandApplication::persistent());
                    };
                    let tabs = match self.nodes.get(group) {
                        Some(Node::Tabs(tabs)) => tabs,
                        _ => {
                            return Err(CommandError::InvalidTarget {
                                reason: "existing singleton panel parent is not a Tabs node",
                            });
                        }
                    };
                    let previous_active = tabs.children.get(tabs.active_index).and_then(|id| {
                        match self.nodes.get(*id) {
                            Some(Node::Panel(panel)) => Some(panel.panel_id),
                            _ => None,
                        }
                    });
                    if !tabs.children.contains(&panel_node) {
                        return Err(CommandError::InvalidTarget {
                            reason: "existing singleton panel is not a child of its Tabs node",
                        });
                    }
                    let was_active = index == tabs.active_index;
                    if !was_active && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) {
                        tabs.active_index = index;
                    }
                    let previous_flyout = self.flyout;
                    if let Some(column) = self.rail_column_for_group(group) {
                        self.flyout = Some(crate::FlyoutState::new(column, group, existing_id));
                    }
                    if !was_active {
                        return Ok(CommandApplication {
                            actual_value: None,
                            inverse: previous_active
                                .map(|panel| DockCommand::ActivateTab { group, panel }),
                            persistent: true,
                        });
                    }
                    if self.flyout != previous_flyout {
                        return Ok(CommandApplication::transient());
                    }
                    return Ok(CommandApplication::persistent());
                }

                let resolved_target = match *target {
                    Some(target) => target,
                    None => self.resolve_open_target(key, metrics)?,
                };
                self.validate_drop_target(resolved_target, metrics)?;
                self.validate_constraints_allowed_for_target(*constraints, resolved_target)?;
                let panel_id = self
                    .panel_instances
                    .insert(PanelInstance::new(key.clone(), *constraints));
                let panel_node = self.nodes.insert(Node::Panel(crate::Panel::new(panel_id)));
                if let DropTarget::RailSlot { column, index } = resolved_target {
                    self.insert_panel_for_rail_slot(panel_node, column, index, metrics)?;
                } else {
                    self.insert_node_for_target(panel_node, resolved_target, metrics, Some(0.5))?;
                    if self.profile == Profile::Photoshop
                        && let DropTarget::Tabs { group, .. } = resolved_target
                        && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group)
                    {
                        tabs.collapsed = false;
                    }
                }
                self.remember_panel_dock_location(panel_node);
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::ClosePanel { panel: panel_id }),
                    persistent: true,
                })
            }
            DockCommand::ClosePanel { panel } => {
                if !self.panel_instances.contains_key(*panel) {
                    return Err(CommandError::MissingPanel { panel: *panel });
                }
                let node = self
                    .find_panel_node(*panel)
                    .ok_or(CommandError::MissingPanel { panel: *panel })?;
                self.remember_panel_dock_location(node);
                self.detach_node(node)?;
                self.remove_subtree(node);
                Ok(CommandApplication::persistent())
            }
            DockCommand::CloseGroup { group } => {
                match self.nodes.get(*group) {
                    Some(Node::Tabs(_)) => {}
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "CloseGroup target must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *group }),
                }
                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                self.remove_subtree(*group);
                Ok(CommandApplication::persistent())
            }
            DockCommand::ActivateTab { group, panel } => {
                let tabs = match self.nodes.get(*group) {
                    Some(Node::Tabs(tabs)) => tabs.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "ActivateTab target must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                let panel_node = self
                    .find_panel_node(*panel)
                    .ok_or(CommandError::MissingPanel { panel: *panel })?;
                let Some(index) = tabs.children.iter().position(|id| *id == panel_node) else {
                    return Err(CommandError::InvalidTarget {
                        reason: "panel is not a tab in the specified group",
                    });
                };
                let previous =
                    tabs.children
                        .get(tabs.active_index)
                        .and_then(|id| match self.nodes.get(*id) {
                            Some(Node::Panel(panel)) => Some(panel.panel_id),
                            _ => None,
                        });
                if index == tabs.active_index {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Tabs(tabs)) = self.nodes.get_mut(*group) {
                    tabs.active_index = index;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: previous.map(|panel| DockCommand::ActivateTab {
                        group: *group,
                        panel,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ReorderTab {
                group,
                panel,
                to_index,
            } => {
                let tabs = match self.nodes.get(*group) {
                    Some(Node::Tabs(tabs)) => tabs.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "ReorderTab target must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                let panel_node = self
                    .find_panel_node(*panel)
                    .ok_or(CommandError::MissingPanel { panel: *panel })?;
                let Some(from_index) = tabs.children.iter().position(|id| *id == panel_node) else {
                    return Err(CommandError::InvalidTarget {
                        reason: "panel is not a tab in the specified group",
                    });
                };
                if *to_index >= tabs.children.len() {
                    return Err(CommandError::InvalidValue {
                        field: "to_index",
                        reason: "tab index is outside the group",
                    });
                }
                if from_index == *to_index {
                    return Ok(CommandApplication::persistent());
                }
                let active_node = tabs.children.get(tabs.active_index).copied();
                let Some(Node::Tabs(current)) = self.nodes.get_mut(*group) else {
                    return Err(CommandError::InvalidTarget {
                        reason: "Tabs node changed unexpectedly",
                    });
                };
                let moved = current.children.remove(from_index);
                current.children.insert(*to_index, moved);
                if let Some(active_node) = active_node
                    && let Some(active_index) =
                        current.children.iter().position(|id| *id == active_node)
                {
                    current.active_index = active_index;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::ReorderTab {
                        group: *group,
                        panel: *panel,
                        to_index: from_index,
                    }),
                    persistent: true,
                })
            }
            DockCommand::MovePanelFromFlyout { panel, target } => {
                if !self.flyout.is_some_and(|flyout| flyout.panel == *panel) {
                    return Err(CommandError::InvalidTarget {
                        reason: "MovePanelFromFlyout requires the dragged panel to own the active flyout",
                    });
                }

                let before = self.clone();
                let mut applied = self.apply_command(
                    &DockCommand::MovePanel {
                        panel: *panel,
                        target: *target,
                    },
                    metrics,
                )?;
                // A valid release is the flyout's terminal transition, even when the
                // target resolves to the panel's current position. Invalid commands
                // return before this point, so the source remains visible on failure.
                self.flyout = None;
                if self.same_persistent_state(&before) {
                    // Closing a flyout after a successful no-op drop is still transient.
                    applied.persistent = false;
                    applied.inverse = None;
                }
                Ok(applied)
            }
            DockCommand::FloatPanelFromFlyout { panel, bounds } => {
                if !self.flyout.is_some_and(|flyout| flyout.panel == *panel) {
                    return Err(CommandError::InvalidTarget {
                        reason: "FloatPanelFromFlyout requires the dragged panel to own the active flyout",
                    });
                }
                let node = self
                    .find_panel_node(*panel)
                    .ok_or(CommandError::MissingPanel { panel: *panel })?;
                let before = self.clone();
                let mut applied = self.apply_command(
                    &DockCommand::Float {
                        node,
                        bounds: *bounds,
                    },
                    metrics,
                )?;
                self.flyout = None;
                if self.same_persistent_state(&before) {
                    applied.persistent = false;
                    applied.inverse = None;
                }
                Ok(applied)
            }
            DockCommand::MovePanel { panel, target } => {
                if matches!(target, DropTarget::GroupTabs { .. }) {
                    return Err(CommandError::InvalidTarget {
                        reason: "GroupTabs target is only valid when merging a whole Tabs group",
                    });
                }
                let panel_node = self
                    .find_panel_node(*panel)
                    .ok_or(CommandError::MissingPanel { panel: *panel })?;
                self.validate_drop_target(*target, metrics)?;
                if let DropTarget::GroupEdge { group: target_group, .. } = *target
                    && self.tab_parent_index(panel_node).is_some_and(|(source_group, _)| source_group == target_group)
                {
                    return Ok(CommandApplication::persistent());
                }
                self.validate_panel_ids_allowed_for_target(&[*panel], *target)?;
                if let (
                    Some((source_group, source_index)),
                    DropTarget::Tabs {
                        group: target_group,
                        index,
                    },
                ) = (self.tab_parent_index(panel_node), *target)
                {
                    if source_group == target_group
                        && (index == source_index || index == source_index + 1)
                    {
                        return Ok(CommandApplication::persistent());
                    }
                    if source_group == target_group {
                        let tabs = match self.nodes.get_mut(source_group) {
                            Some(Node::Tabs(tabs)) => tabs,
                            _ => {
                                return Err(CommandError::InvalidTarget {
                                    reason: "panel parent is not a Tabs node",
                                });
                            }
                        };
                        let active_node = tabs.children.get(tabs.active_index).copied();
                        let moved = tabs.children.remove(source_index);
                        let insert_index = if index > source_index {
                            index - 1
                        } else {
                            index
                        };
                        tabs.children
                            .insert(insert_index.min(tabs.children.len()), moved);
                        if let Some(active_node) = active_node
                            && let Some(active_index) =
                                tabs.children.iter().position(|id| *id == active_node)
                        {
                            tabs.active_index = active_index;
                        }
                        return Ok(CommandApplication {
                            actual_value: None,
                            inverse: Some(DockCommand::ReorderTab {
                                group: source_group,
                                panel: *panel,
                                to_index: source_index,
                            }),
                            persistent: true,
                        });
                    }
                }
                let inverse = self
                    .tab_parent_index(panel_node)
                    .and_then(|(group, index)| {
                        let source_group_has_other_tabs = matches!(
                            self.nodes.get(group),
                            Some(Node::Tabs(tabs)) if tabs.children.len() > 1
                        );
                        source_group_has_other_tabs.then_some(DockCommand::MovePanel {
                            panel: *panel,
                            target: DropTarget::Tabs { group, index },
                        })
                    });
                if let DropTarget::RailSlot { column, index } = *target {
                    let order = self.column_display_order(column)?;
                    if let Some(source_index) = order.iter().position(|item| item.panel == *panel)
                        && (index == source_index || index == source_index + 1)
                    {
                        return Ok(CommandApplication::persistent());
                    }
                    self.remember_panel_dock_location(panel_node);
                    let moved =
                        self.insert_panel_for_rail_slot(panel_node, column, index, metrics)?;
                    if !moved {
                        return Ok(CommandApplication::persistent());
                    }
                    self.remember_panel_dock_location(panel_node);
                    return Ok(CommandApplication {
                        actual_value: None,
                        inverse,
                        persistent: true,
                    });
                }
                self.remember_panel_dock_location(panel_node);
                self.detach_node(panel_node)?;
                self.insert_node_for_target(panel_node, *target, metrics, Some(0.5))?;

                // In the Photoshop profile a center/tab drop is an explicit
                // reveal action: the target becomes expanded after a successful
                // insertion. Same-group reorder/no-op paths returned above, so
                // a self-drop cannot implicitly change collapse state.
                if self.profile == crate::Profile::Photoshop
                    && let DropTarget::Tabs {
                        group: target_group,
                        ..
                    } = *target
                    && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(target_group)
                {
                    tabs.collapsed = false;
                }

                self.remember_panel_dock_location(panel_node);
                Ok(CommandApplication {
                    actual_value: None,
                    inverse,
                    persistent: true,
                })
            }
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
            DockCommand::ResizeGroups {
                column,
                boundary_index,
                delta,
                available_height,
            } => {
                if !delta.is_finite() {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "group boundary delta must be finite",
                    });
                }
                if !available_height.is_finite() || *available_height <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "available group height must be finite and positive",
                    });
                }
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "ResizeGroups target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if *boundary_index >= current.groups.len().saturating_sub(1) {
                    return Err(CommandError::InvalidTarget {
                        reason: "group boundary index does not identify adjacent groups",
                    });
                }
                let left_group = current.groups[*boundary_index];
                let right_group = current.groups[*boundary_index + 1];
                let heights =
                    self.group_heights_for_available_height(*column, *available_height, metrics)?;
                let minimums = self.group_minimum_heights(*column, metrics)?;
                // group_minimum_heights already includes the effective fixed
                // height for collapsed groups; do not add that floor a second time.
                let required_height = minimums.iter().sum::<f32>();
                if !required_height.is_finite() || *available_height < required_height {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "available height is below the sum of group minimum heights",
                    });
                }
                if self.group_is_collapsed(left_group) || self.group_is_collapsed(right_group) {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }

                let left_index = *boundary_index;
                let right_index = left_index + 1;
                let lower_delta = minimums[left_index] - heights[left_index];
                let upper_delta = heights[right_index] - minimums[right_index];
                if lower_delta > upper_delta {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "available height cannot satisfy adjacent group minimums",
                    });
                }
                let requested_delta = (*delta).clamp(lower_delta, upper_delta);
                if requested_delta == 0.0 {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }
                if current.groups.iter().enumerate().any(|(index, group)| {
                    !self.group_is_collapsed(*group)
                        && heights[index] + PIXEL_TOLERANCE < minimums[index]
                }) {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "current group ratios place an expanded group below its minimum; reset ratios before resizing",
                    });
                }

                // Ratios are weights among expanded groups. Transfer weight only
                // between this pair so every non-adjacent group's ratio remains
                // unchanged and the pair keeps its combined weight.
                let expanded_weight_sum: f32 = current
                    .groups
                    .iter()
                    .enumerate()
                    .filter(|(_, group)| !self.group_is_collapsed(**group))
                    .map(|(index, _)| current.group_ratios[index])
                    .sum();
                let expanded_height: f32 = current
                    .groups
                    .iter()
                    .enumerate()
                    .filter(|(_, group)| !self.group_is_collapsed(**group))
                    .map(|(index, _)| heights[index])
                    .sum();
                if expanded_weight_sum <= f32::EPSILON || expanded_height <= 0.0 {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }

                let weight_delta = requested_delta * expanded_weight_sum / expanded_height;
                let mut new_ratios = current.group_ratios.clone();
                new_ratios[left_index] += weight_delta;
                new_ratios[right_index] -= weight_delta;
                if new_ratios[left_index] < 0.0 || new_ratios[right_index] < 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "constrained resize produced a negative group weight",
                    });
                }
                let final_heights = self.group_heights_for_ratios(
                    *column,
                    &new_ratios,
                    *available_height,
                    metrics,
                )?;
                let actual_delta = final_heights[left_index] - heights[left_index];
                if actual_delta == 0.0 || new_ratios == current.group_ratios {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.group_ratios = new_ratios;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::GroupBoundaryDelta(actual_delta)),
                    inverse: Some(DockCommand::RestoreGroupRatios {
                        column: *column,
                        ratios: current.group_ratios,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeColumn { column, width } => {
                if !width.is_finite() || *width <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "width must be finite and positive",
                    });
                }
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "ResizeColumn target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                let old_width = current.width;
                if let Some((side, column_index)) = self.main_column_position(*column) {
                    let main_columns = self.main_columns(side)?;
                    let canvas_adjacent = match side {
                        DockSide::Left => column_index + 1 == main_columns.len(),
                        DockSide::Right => column_index == 0,
                    };
                    if !canvas_adjacent {
                        return Err(CommandError::InvalidTarget {
                            reason: "interior main columns must be resized through ResizeColumnBoundary",
                        });
                    }
                }
                let min_width = self.minimum_column_width(*column, metrics)?;
                let panel_max = self.maximum_column_width(*column)?;
                let mut max_width = panel_max.unwrap_or(f32::MAX);

                // When the canvas has a measured remainder, resizing an outer
                // main-dock column consumes or returns that remainder. Preserve
                // the mandatory min_canvas_width rather than letting the UI
                // render a canvas narrower than Core's own invariant.
                if self.main_column_position(*column).is_some()
                    && let Some(bounds) = self.canvas.last_known_bounds
                {
                    let room = (bounds.size.width - metrics.min_canvas_width).max(0.0);
                    let canvas_cap = old_width + room;
                    if canvas_cap.is_finite() {
                        max_width = max_width.min(canvas_cap);
                    }
                }
                if max_width < min_width {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "column minimum width exceeds its effective maximum width",
                    });
                }
                let actual_width = match current.mode {
                    ColumnMode::Expanded => width.clamp(min_width, max_width),
                    ColumnMode::IconsAndLabels => metrics.min_column_width,
                    ColumnMode::IconsOnly => metrics.rail_width,
                };
                if !actual_width.is_finite() || actual_width <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "effective width is not finite and positive",
                    });
                }
                let actual_delta = actual_width - old_width;
                if actual_delta != 0.0 {
                    if let Some(Node::Column(column)) = self.nodes.get_mut(*column) {
                        column.width = actual_width;
                        if current.mode == ColumnMode::Expanded {
                            column.last_expanded_width = Some(actual_width);
                        }
                    }
                    self.adjust_canvas_for_column_resize(*column, actual_delta, metrics)?;
                    self.sync_main_column_ratios(*column)?;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::ColumnWidth(actual_width)),
                    inverse: (actual_delta != 0.0).then_some(DockCommand::ResizeColumn {
                        column: *column,
                        width: old_width,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeColumnBoundary {
                side,
                boundary_index,
                delta,
            } => {
                if !delta.is_finite() {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "column boundary delta must be finite",
                    });
                }
                let columns = self.main_columns(*side)?;
                if *boundary_index >= columns.len().saturating_sub(1) {
                    return Err(CommandError::InvalidTarget {
                        reason: "column boundary index does not identify adjacent columns",
                    });
                }
                let left_id = columns[*boundary_index];
                let right_id = columns[*boundary_index + 1];
                let left = match self.nodes.get(left_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "main dock contains a non-Column child",
                        });
                    }
                };
                let right = match self.nodes.get(right_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => {
                        return Err(CommandError::InvalidTarget {
                            reason: "main dock contains a non-Column child",
                        });
                    }
                };
                let actual_delta = if left.mode.is_rail() || right.mode.is_rail() {
                    0.0
                } else {
                    let left_min = self.minimum_column_width(left_id, metrics)?;
                    let right_min = self.minimum_column_width(right_id, metrics)?;
                    let left_max = self.maximum_column_width(left_id)?.unwrap_or(f32::INFINITY);
                    let right_max = self
                        .maximum_column_width(right_id)?
                        .unwrap_or(f32::INFINITY);
                    let lower_delta = (left_min - left.width).max(right.width - right_max);
                    let upper_delta = (right.width - right_min).min(left_max - left.width);
                    if lower_delta > upper_delta {
                        return Err(CommandError::InvalidValue {
                            field: "delta",
                            reason: "adjacent columns have incompatible min/max width constraints",
                        });
                    }
                    (*delta).clamp(lower_delta, upper_delta)
                };
                if actual_delta != 0.0 {
                    let new_left = left.width + actual_delta;
                    let new_right = right.width - actual_delta;
                    if !new_left.is_finite()
                        || !new_right.is_finite()
                        || new_left <= 0.0
                        || new_right <= 0.0
                    {
                        return Err(CommandError::InvalidValue {
                            field: "delta",
                            reason: "resized column width is not finite and positive",
                        });
                    }
                    if let Some(Node::Column(column)) = self.nodes.get_mut(left_id) {
                        column.width = new_left;
                        column.last_expanded_width = Some(new_left);
                    }
                    if let Some(Node::Column(column)) = self.nodes.get_mut(right_id) {
                        column.width = new_right;
                        column.last_expanded_width = Some(new_right);
                    }
                    self.sync_main_column_ratios(left_id)?;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::ColumnBoundaryDelta(actual_delta)),
                    inverse: (actual_delta != 0.0).then_some(DockCommand::ResizeColumnBoundary {
                        side: *side,
                        boundary_index: *boundary_index,
                        delta: -actual_delta,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeSplitColumnBoundary {
                split,
                boundary_index,
                delta,
            } => {
                if !delta.is_finite() {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "split column boundary delta must be finite",
                    });
                }
                let current_split = match self.nodes.get(*split) {
                    Some(Node::Split(value)) if value.axis == Axis::X => value.clone(),
                    Some(Node::Split(_)) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "ResizeSplitColumnBoundary target must be a horizontal Split(X)",
                        });
                    }
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "ResizeSplitColumnBoundary target must be a Split node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *split }),
                };
                if current_split.children.len() < 2
                    || current_split
                        .children
                        .iter()
                        .any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_))))
                {
                    return Err(CommandError::InvalidTarget {
                        reason: "ResizeSplitColumnBoundary requires a Split(X) containing only Columns",
                    });
                }
                if *boundary_index >= current_split.children.len() - 1 {
                    return Err(CommandError::InvalidTarget {
                        reason: "split column boundary index does not identify adjacent columns",
                    });
                }

                let left_id = current_split.children[*boundary_index];
                let right_id = current_split.children[*boundary_index + 1];
                let left = match self.nodes.get(left_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => return Err(CommandError::InvalidTarget {
                        reason: "split column boundary references a non-Column node",
                    }),
                };
                let right = match self.nodes.get(right_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => return Err(CommandError::InvalidTarget {
                        reason: "split column boundary references a non-Column node",
                    }),
                };

                let actual_delta = if left.mode.is_rail() || right.mode.is_rail() {
                    0.0
                } else {
                    let left_min = self.minimum_column_width(left_id, metrics)?;
                    let right_min = self.minimum_column_width(right_id, metrics)?;
                    let left_max = self.maximum_column_width(left_id)?.unwrap_or(f32::INFINITY);
                    let right_max = self
                        .maximum_column_width(right_id)?
                        .unwrap_or(f32::INFINITY);
                    let lower_delta = (left_min - left.width).max(right.width - right_max);
                    let upper_delta = (right.width - right_min).min(left_max - left.width);
                    if lower_delta > upper_delta {
                        return Err(CommandError::InvalidValue {
                            field: "delta",
                            reason: "adjacent columns have incompatible min/max width constraints",
                        });
                    }
                    (*delta).clamp(lower_delta, upper_delta)
                };

                if actual_delta != 0.0 {
                    let new_left = left.width + actual_delta;
                    let new_right = right.width - actual_delta;
                    if !new_left.is_finite()
                        || !new_right.is_finite()
                        || new_left <= 0.0
                        || new_right <= 0.0
                    {
                        return Err(CommandError::InvalidValue {
                            field: "delta",
                            reason: "resized split column width is not finite and positive",
                        });
                    }
                    if let Some(Node::Column(column)) = self.nodes.get_mut(left_id) {
                        column.width = new_left;
                        column.last_expanded_width = Some(new_left);
                    }
                    if let Some(Node::Column(column)) = self.nodes.get_mut(right_id) {
                        column.width = new_right;
                        column.last_expanded_width = Some(new_right);
                    }

                    // Split(X) of Columns is laid out from each Column's requested
                    // width. Keep ratios in sync so the tree remains meaningful if
                    // another layout mode later consumes the stored ratios.
                    let widths: Vec<f32> = current_split
                        .children
                        .iter()
                        .map(|child_id| match self.nodes.get(*child_id) {
                            Some(Node::Column(column)) => Ok(column.width),
                            _ => Err(CommandError::InvalidTarget {
                                reason: "split column boundary references a non-Column node",
                            }),
                        })
                        .collect::<Result<_, _>>()?;
                    let total_width = widths.iter().sum::<f32>();
                    if !total_width.is_finite() || total_width <= 0.0 {
                        return Err(CommandError::InvalidValue {
                            field: "delta",
                            reason: "split column widths must have a finite positive total",
                        });
                    }
                    if let Some(Node::Split(split_node)) = self.nodes.get_mut(*split) {
                        for (ratio, width) in split_node.ratios.iter_mut().zip(widths) {
                            *ratio = width / total_width;
                        }
                    }
                }

                Ok(CommandApplication {
                    actual_value: Some(CommandValue::ColumnBoundaryDelta(actual_delta)),
                    inverse: (actual_delta != 0.0).then_some(
                        DockCommand::ResizeSplitColumnBoundary {
                            split: *split,
                            boundary_index: *boundary_index,
                            delta: -actual_delta,
                        },
                    ),
                    persistent: true,
                })
            }
            DockCommand::ResetColumnRatios {
                column,
                available_height,
            } => {
                if !available_height.is_finite() || *available_height <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "available group height must be finite and positive",
                    });
                }
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "ResetColumnRatios target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                // Validate the supplied height against fixed collapsed strips
                // and expanded minimums even when the reset itself becomes a no-op.
                let _current_heights =
                    self.group_heights_for_available_height(*column, *available_height, metrics)?;
                if current
                    .groups
                    .iter()
                    .all(|group| self.group_is_collapsed(*group))
                {
                    return Ok(CommandApplication::persistent());
                }
                let heights = self.equal_group_heights(*column, *available_height, metrics)?;
                let ratios = self.group_ratios_from_heights(*column, &heights)?;
                if ratios == current.group_ratios {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.group_ratios = ratios.clone();
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::GroupRatios(ratios)),
                    inverse: Some(DockCommand::RestoreGroupRatios {
                        column: *column,
                        ratios: current.group_ratios,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResetMainColumnRatios { side } => {
                let Some(area) = self.main_area(*side).copied() else {
                    return Ok(CommandApplication::persistent());
                };
                let root = self.nodes.get(area.root).cloned();
                match root {
                    Some(Node::Column(_)) => Ok(CommandApplication::persistent()),
                    Some(Node::Split(split)) if split.axis == Axis::X => {
                        if split.children.iter().any(|child| {
                            !matches!(self.nodes.get(*child), Some(Node::Column(_)))
                        }) {
                            return Err(CommandError::InvalidTarget {
                                reason: "main horizontal split contains a non-Column child",
                            });
                        }
                        let first_column = split.children.first().copied().ok_or(
                            CommandError::InvalidTarget {
                                reason: "main horizontal split has no columns",
                            },
                        )?;
                        self.reset_main_column_widths(*side, metrics)?;
                        self.sync_main_column_ratios(first_column)?;
                        Ok(CommandApplication::persistent())
                    }
                    Some(_) => Err(CommandError::InvalidTarget {
                        reason: "main dock root is not a horizontal column split",
                    }),
                    None => Err(CommandError::InvalidTarget {
                        reason: "main dock root is missing",
                    }),
                }
            }
            DockCommand::RestoreGroupRatios { column, ratios } => {
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.group_ratios.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "RestoreGroupRatios target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                let count = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.groups.len(),
                    _ => 0,
                };
                if ratios.len() != count {
                    return Err(CommandError::InvalidValue {
                        field: "ratios",
                        reason: "ratio count must match group count",
                    });
                }
                if let Some(reason) = ratio_issues(ratios, count).first() {
                    return Err(CommandError::InvalidValue {
                        field: "ratios",
                        reason,
                    });
                }
                if current == *ratios {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.group_ratios = ratios.clone();
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::GroupRatios(ratios.clone())),
                    inverse: Some(DockCommand::RestoreGroupRatios {
                        column: *column,
                        ratios: current,
                    }),
                    persistent: true,
                })
            }
            DockCommand::Float { node, bounds } => {
                // Bounds use model-space logical pixels. Visible-screen clamping
                // belongs to the geometry/render adapter, not to Core/GPUI.
                if !self.nodes.contains_key(*node) {
                    return Err(CommandError::MissingNode { node: *node });
                }
                bounds
                    .validate_non_empty()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "bounds",
                        reason: geometry_error_reason(error),
                    })?;
                let moving_panels = self.panel_ids_in_subtree(*node);
                self.validate_panel_ids_allowed_for_area(&moving_panels, AllowedArea::FLOATING)?;
                let supports_photoshop_floating_root = match self.nodes.get(*node) {
                    Some(Node::Panel(_) | Node::Tabs(_)) => true,
                    Some(Node::Column(column)) => column
                        .groups
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Tabs(_)))),
                    Some(Node::Split(split)) if split.axis == Axis::X => split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))),
                    _ => false,
                };
                if self.profile == Profile::Photoshop && !supports_photoshop_floating_root {
                    return Err(CommandError::UnsupportedForProfile {
                        profile: self.profile,
                        reason: "only a panel, tab group, column, or horizontal composition of columns can become a floating root",
                    });
                }
                let next_z = self.next_floating_z_order()?;
                let panel_root = matches!(self.nodes.get(*node), Some(Node::Panel(_)));
                self.remember_subtree_dock_locations(*node);
                self.detach_node(*node)?;
                let root = if panel_root && self.profile == Profile::Photoshop {
                    self.nodes.insert(Node::Tabs(crate::Tabs {
                        children: vec![*node],
                        active_index: 0,
                        collapsed: false,
                    }))
                } else {
                    *node
                };
                let area_id = self.dock_areas.insert(DockArea::floating(root));
                self.floating_groups
                    .insert(FloatingGroup::new(area_id, *bounds, next_z));
                Ok(CommandApplication::persistent())
            }
            DockCommand::FloatGroupAsColumn { group, bounds } => {
                self.float_group_as_column(*group, *bounds, metrics)
            }
            DockCommand::FloatPanelAsGroup { panel, bounds } => {
                self.float_panel_as_group(*panel, *bounds, metrics)
            }
            DockCommand::MoveFloating { group, origin } => {
                // Do not clamp to a monitor/work-area here: the adapter applies
                // its visible-area policy; Core stores finite logical coordinates.
                if !origin.is_finite() {
                    return Err(CommandError::InvalidValue {
                        field: "origin",
                        reason: "origin coordinates must be finite",
                    });
                }
                let floating = self
                    .floating_groups
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                let bounds = Rect::new(*origin, floating.bounds.size);
                bounds
                    .validate_non_empty()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "origin",
                        reason: geometry_error_reason(error),
                    })?;
                if floating.bounds.origin == *origin {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds.origin = *origin;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::MoveFloating {
                        group: *group,
                        origin: floating.bounds.origin,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeFloating { group, size } => {
                size.validate_positive()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "size",
                        reason: geometry_error_reason(error),
                    })?;
                let floating = self
                    .floating_groups
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                let root = self
                    .dock_areas
                    .get(floating.area)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "floating group references a missing dock area",
                    })?
                    .root;
                let minimum = self.minimum_floating_size(root, metrics)?;
                let actual = Size::new(
                    size.width.max(minimum.width),
                    size.height.max(minimum.height),
                );
                let new_bounds = Rect::new(floating.bounds.origin, actual);
                new_bounds
                    .validate_non_empty()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "size",
                        reason: geometry_error_reason(error),
                    })?;
                if actual == floating.bounds.size {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::FloatingSize(actual)),
                        inverse: None,
                        persistent: true,
                    });
                }
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds = new_bounds;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::FloatingSize(actual)),
                    inverse: Some(DockCommand::ResizeFloating {
                        group: *group,
                        size: floating.bounds.size,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeFloatingBounds { group, bounds } => {
                bounds.validate_non_empty().map_err(|error| CommandError::InvalidValue {
                    field: "bounds",
                    reason: geometry_error_reason(error),
                })?;
                let floating = self
                    .floating_groups
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                let root = self
                    .dock_areas
                    .get(floating.area)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "floating group references a missing dock area",
                    })?
                    .root;
                let minimum = self.minimum_floating_size(root, metrics)?;
                let actual_bounds = Rect::new(
                    bounds.origin,
                    Size::new(
                        bounds.size.width.max(minimum.width),
                        bounds.size.height.max(minimum.height),
                    ),
                );
                actual_bounds.validate_non_empty().map_err(|error| {
                    CommandError::InvalidValue {
                        field: "bounds",
                        reason: geometry_error_reason(error),
                    }
                })?;
                if actual_bounds == floating.bounds {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds = actual_bounds;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::ResizeFloatingBounds {
                        group: *group,
                        bounds: floating.bounds,
                    }),
                    persistent: true,
                })
            }
            DockCommand::RaiseFloating { group } => {
                let current = self
                    .floating_groups
                    .get(*group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                let max_other = self
                    .floating_groups
                    .iter()
                    .filter(|(other_id, _)| *other_id != *group)
                    .map(|(_, value)| value.z_order)
                    .max();
                if max_other.is_none_or(|max| current.z_order > max) {
                    return Ok(CommandApplication::persistent());
                }
                let max_z = self
                    .floating_groups
                    .iter()
                    .map(|(_, value)| value.z_order)
                    .max()
                    .unwrap_or(-1);
                let next_z = max_z.checked_add(1).ok_or(CommandError::InvalidValue {
                    field: "z_order",
                    reason: "floating z-order overflow",
                })?;
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.z_order = next_z;
                }
                Ok(CommandApplication::persistent())
            }
            DockCommand::DockFloating { group, target } => {
                let root = self
                    .floating_groups
                    .get(*group)
                    .and_then(|floating| self.dock_areas.get(floating.area).map(|area| area.root))
                    .ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                self.validate_drop_target(*target, metrics)?;
                let panel_nodes = self.panel_nodes_in_subtree(root);
                let moving_panels = self.panel_ids_in_subtree(root);
                self.validate_panel_ids_allowed_for_target(&moving_panels, *target)?;
                self.dock_floating_group(*group, *target, metrics)?;
                for panel_node in panel_nodes {
                    self.remember_panel_dock_location(panel_node);
                }
                Ok(CommandApplication::persistent())
            }
            DockCommand::CollapseGroup { group, collapsed } => {
                let old = match self.nodes.get(*group) {
                    Some(Node::Tabs(tabs)) => tabs.collapsed,
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "CollapseGroup target must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                if old == *collapsed {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Tabs(tabs)) = self.nodes.get_mut(*group) {
                    tabs.collapsed = *collapsed;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::CollapseGroup {
                        group: *group,
                        collapsed: old,
                    }),
                    persistent: true,
                })
            }
            DockCommand::SetColumnMode { column, mode } => {
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "SetColumnMode target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if current.mode == *mode {
                    return Ok(CommandApplication::persistent());
                }
                let destination_area = if mode.is_rail() {
                    // Rail geometry represents each Tabs child as a stable panel
                    // entry, so reject malformed/non-tab columns before changing
                    // the mode instead of committing a layout that cannot render.
                    self.column_display_order(*column)?;
                    AllowedArea::ICON_RAIL
                } else {
                    AllowedArea::MAIN_DOCK
                };
                let moving_panels: Vec<_> = current
                    .groups
                    .iter()
                    .flat_map(|group| self.panel_ids_in_subtree(*group))
                    .collect();
                self.validate_panel_ids_allowed_for_area(&moving_panels, destination_area)?;
                let width = match *mode {
                    ColumnMode::IconsOnly => metrics.rail_width,
                    ColumnMode::IconsAndLabels => metrics.min_column_width,
                    ColumnMode::Expanded if current.mode.is_rail() => {
                        let minimum = self.minimum_expanded_column_width(*column, metrics)?;
                        let maximum = self
                            .maximum_expanded_column_width(*column)?
                            .unwrap_or(f32::MAX);
                        if maximum < minimum {
                            return Err(CommandError::InvalidValue {
                                field: "width",
                                reason: "expanded column minimum width exceeds its panel maximum",
                            });
                        }
                        current
                            .last_expanded_width
                            .unwrap_or(metrics.default_column_width)
                            .clamp(minimum, maximum)
                    }
                    ColumnMode::Expanded => current
                        .width
                        .max(self.minimum_column_width(*column, metrics)?),
                };
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.mode = *mode;
                    value.width = width;
                    value.last_expanded_width =
                        if current.mode == ColumnMode::Expanded && mode.is_rail() {
                            Some(current.width)
                        } else if *mode == ColumnMode::Expanded {
                            Some(width)
                        } else {
                            current.last_expanded_width
                        };
                }
                if !mode.is_rail() && self.flyout.is_some_and(|flyout| flyout.column == *column) {
                    self.flyout = None;
                }
                self.sync_main_column_ratios(*column)?;
                Ok(CommandApplication::persistent())
            }
            DockCommand::OpenFlyout { column, panel } => {
                match self.nodes.get(*column) {
                    Some(Node::Column(value)) if value.mode.is_rail() => {}
                    Some(Node::Column(_)) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "flyout column must use a rail mode",
                        });
                    }
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "OpenFlyout target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                }

                let belongs_to_main_dock = [DockSide::Left, DockSide::Right]
                    .into_iter()
                    .filter_map(|side| self.main_area(side).map(|area| area.root))
                    .any(|root| self.is_descendant_or_same(root, *column));
                if !belongs_to_main_dock {
                    return Err(CommandError::InvalidTarget {
                        reason: "flyout column must belong to a main dock",
                    });
                }

                let item = self
                    .column_display_order(*column)?
                    .into_iter()
                    .find(|item| item.panel == *panel)
                    .ok_or(CommandError::InvalidTarget {
                        reason: "panel does not have an icon in the requested rail",
                    })?;
                let flyout = crate::FlyoutState::new(*column, item.group, *panel);
                if self.flyout == Some(flyout) {
                    if self.flyout_auto_collapse {
                        self.flyout = None;
                    }
                    return Ok(CommandApplication::transient());
                }

                // Selecting an inactive rail icon must not mutate the group's
                // persistent active_index. Another icon replaces the current flyout.
                self.flyout = Some(flyout);
                Ok(CommandApplication::transient())
            }
            DockCommand::CloseFlyout => {
                if self.flyout.is_some() {
                    self.flyout = None;
                }
                Ok(CommandApplication::transient())
            }
            DockCommand::DismissFlyout => {
                if self.flyout_auto_collapse {
                    self.flyout = None;
                }
                Ok(CommandApplication::transient())
            }
            DockCommand::SetFlyoutAutoCollapse { enabled } => {
                self.flyout_auto_collapse = *enabled;
                Ok(CommandApplication::transient())
            }
            DockCommand::ResizeFlyout { panel, width } => {
                if !width.is_finite() || *width <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "flyout width must be finite and positive",
                    });
                }
                let Some(flyout) = self.flyout else {
                    return Err(CommandError::InvalidTarget {
                        reason: "ResizeFlyout requires an active flyout",
                    });
                };
                if flyout.panel != *panel {
                    return Err(CommandError::InvalidTarget {
                        reason: "ResizeFlyout panel must match the active flyout",
                    });
                }
                let instance = self
                    .panel_instances
                    .get(*panel)
                    .ok_or(CommandError::MissingPanel { panel: *panel })?;
                let key = instance.key.clone();
                let constraints = instance.constraints;
                let max_width = constraints
                    .max_size
                    .map(|size| size.width)
                    .unwrap_or(f32::MAX);
                if !width.is_finite() {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "flyout width must be finite",
                    });
                }
                let actual_width = (*width).clamp(constraints.min_size.width, max_width);
                self.flyout_width_order.retain(|remembered| *remembered != key);
                self.flyout_width_order.push_back(key.clone());
                self.flyout_widths.insert(key, actual_width);
                while self.flyout_width_order.len() > MAX_REMEMBERED_DOCK_LOCATIONS {
                    if let Some(expired) = self.flyout_width_order.pop_front() {
                        self.flyout_widths.remove(&expired);
                    }
                }
                Ok(CommandApplication::transient())
            }
            DockCommand::SetChrome { mode } => {
                let old = self.chrome;
                if old == *mode {
                    return Ok(CommandApplication::persistent());
                }
                self.chrome = *mode;
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::SetChrome { mode: old }),
                    persistent: true,
                })
            }
        }
    }
}
