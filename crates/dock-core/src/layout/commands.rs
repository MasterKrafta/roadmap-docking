//! Command transactions, dispatch, and history snapshot restoration.

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
        candidate
            .validation_cache
            .set(Some((candidate.version, *metrics)));

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
        candidate
            .validation_cache
            .set(Some((next_version, *metrics)));
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
            DockCommand::Float { .. }
            | DockCommand::FloatGroupAsColumn { .. }
            | DockCommand::FloatPanelAsGroup { .. }
            | DockCommand::MoveFloating { .. }
            | DockCommand::ResizeFloating { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::RaiseFloating { .. }
            | DockCommand::DockFloating { .. } => self.apply_floating_command(command, metrics),
            DockCommand::OpenPanel { .. }
            | DockCommand::ClosePanel { .. }
            | DockCommand::CloseGroup { .. }
            | DockCommand::ActivateTab { .. }
            | DockCommand::ReorderTab { .. } => self.apply_panel_command(command, metrics),
            DockCommand::MovePanelFromFlyout { .. }
            | DockCommand::FloatPanelFromFlyout { .. }
            | DockCommand::MovePanel { .. } => self.apply_panel_transfer_command(command, metrics),
            DockCommand::MoveGroup { .. } | DockCommand::MoveGroupToMainColumn { .. } => {
                self.apply_group_transfer_command(command, metrics)
            }
            DockCommand::MergeGroup { .. } | DockCommand::SplitGroup { .. } => {
                self.apply_group_command(command, metrics)
            }
            DockCommand::ResizeGroups { .. }
            | DockCommand::ResetColumnRatios { .. }
            | DockCommand::RestoreGroupRatios { .. } => {
                self.apply_group_resize_command(command, metrics)
            }
            DockCommand::ResizeColumn { .. }
            | DockCommand::ResizeColumnBoundary { .. }
            | DockCommand::ResizeSplitColumnBoundary { .. }
            | DockCommand::ResetMainColumnRatios { .. } => {
                self.apply_column_resize_command(command, metrics)
            }
            DockCommand::CollapseGroup { .. }
            | DockCommand::SetColumnMode { .. }
            | DockCommand::SetChrome { .. } => self.apply_layout_state_command(command, metrics),
            DockCommand::OpenFlyout { .. }
            | DockCommand::CloseFlyout
            | DockCommand::DismissFlyout
            | DockCommand::SetFlyoutAutoCollapse { .. }
            | DockCommand::ResizeFlyout { .. } => self.apply_flyout_command(command),
        }
    }

    pub(super) fn same_state(&self, other: &Self) -> bool {
        self.same_persistent_state(other)
            && self.flyout == other.flyout
            && self.flyout_auto_collapse == other.flyout_auto_collapse
            && self.flyout_widths == other.flyout_widths
            && self.flyout_width_order == other.flyout_width_order
    }

    pub(super) fn same_persistent_state(&self, other: &Self) -> bool {
        self.nodes.iter().eq(other.nodes.iter())
            && self.panel_instances.iter().eq(other.panel_instances.iter())
            && self.dock_areas.iter().eq(other.dock_areas.iter())
            && self.main_areas == other.main_areas
            && self.floating_groups.iter().eq(other.floating_groups.iter())
            && self.last_dock_locations == other.last_dock_locations
            && self.canvas == other.canvas
            && self.profile == other.profile
            && self.version == other.version
            && self.chrome == other.chrome
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
                    affected.extend(
                        layout
                            .main_areas
                            .iter()
                            .flatten()
                            .filter_map(|area_id| layout.dock_area(*area_id).map(|area| area.root)),
                    );
                    affected.extend(
                        layout
                            .floating_groups
                            .values()
                            .filter_map(|group| layout.dock_area(group.area).map(|area| area.root)),
                    );
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

    pub(super) fn apply_layout_state_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
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
            _ => Err(CommandError::InvalidTarget {
                reason: "layout state dispatcher received an unsupported command",
            }),
        }
    }
}

impl Layout {
    /// Captures a session-only history checkpoint without transient flyout UI.
    pub(crate) fn history_snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.flyout = None;
        snapshot
    }

    /// Restores a previously validated history checkpoint as a new model revision.
    ///
    /// The model revision is monotonic even though the persistent state moves
    /// backward/forward. Transient flyout state is not restored, and the current
    /// flyout auto-collapse preference remains session-local.
    pub(crate) fn restore_history_snapshot(
        &mut self,
        snapshot: &Self,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let next_version = self
            .version
            .checked_add(1)
            .ok_or(CommandError::InvalidValue {
                field: "version",
                reason: "layout version overflow",
            })?;

        // Rebuild into a clone of the *current* arenas instead of replacing them
        // with the historical arenas. Keys for entities that still exist in
        // both states stay stable. Removed entities receive fresh generations,
        // so undo cannot make stale SlotMap handles valid again (the ABA problem).
        let mut candidate = self.clone();
        candidate.flyout = None;
        candidate.flyout_auto_collapse = self.flyout_auto_collapse;

        let wanted_panel_ids: HashSet<_> = snapshot.panel_instances.keys().collect();
        for id in candidate.panel_instances.keys().collect::<Vec<_>>() {
            if !wanted_panel_ids.contains(&id) {
                candidate.panel_instances.remove(id);
            }
        }
        let mut panel_map = HashMap::<PanelId, PanelId>::new();
        for (old_id, instance) in snapshot.panel_instances.iter() {
            let new_id = if let Some(current) = candidate.panel_instances.get_mut(old_id) {
                let runtime_state = current.state;
                *current = instance.clone();
                current.state = runtime_state;
                old_id
            } else {
                candidate.panel_instances.insert(instance.clone())
            };
            panel_map.insert(old_id, new_id);
        }

        let wanted_node_ids: HashSet<_> = snapshot.nodes.keys().collect();
        for id in candidate.nodes.keys().collect::<Vec<_>>() {
            if !wanted_node_ids.contains(&id) {
                candidate.nodes.remove(id);
            }
        }

        fn rebuild_node(
            snapshot: &Layout,
            target_id: NodeId,
            target_nodes: &mut SlotMap<NodeId, Node>,
            panel_map: &HashMap<PanelId, PanelId>,
            node_map: &mut HashMap<NodeId, NodeId>,
            visiting: &mut HashSet<NodeId>,
        ) -> Result<NodeId, CommandError> {
            if let Some(mapped) = node_map.get(&target_id) {
                return Ok(*mapped);
            }
            if !visiting.insert(target_id) {
                return Err(CommandError::InvalidLayout {
                    errors: vec![ValidationError::NodeCycle {
                        from: target_id,
                        to: target_id,
                    }],
                });
            }

            let source = snapshot
                .nodes
                .get(target_id)
                .cloned()
                .ok_or(CommandError::MissingNode { node: target_id })?;
            let rebuilt = match source {
                Node::Split(split) => {
                    let children = split
                        .children
                        .iter()
                        .map(|child| {
                            rebuild_node(
                                snapshot,
                                *child,
                                target_nodes,
                                panel_map,
                                node_map,
                                visiting,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    Node::Split(Split {
                        axis: split.axis,
                        children,
                        ratios: split.ratios,
                    })
                }
                Node::Tabs(tabs) => {
                    let children = tabs
                        .children
                        .iter()
                        .map(|child| {
                            rebuild_node(
                                snapshot,
                                *child,
                                target_nodes,
                                panel_map,
                                node_map,
                                visiting,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    Node::Tabs(Tabs {
                        children,
                        active_index: tabs.active_index,
                        collapsed: tabs.collapsed,
                    })
                }
                Node::Panel(panel) => {
                    let panel_id = panel_map.get(&panel.panel_id).copied().ok_or(
                        CommandError::MissingPanel {
                            panel: panel.panel_id,
                        },
                    )?;
                    Node::Panel(Panel::new(panel_id))
                }
                Node::Column(column) => {
                    let groups = column
                        .groups
                        .iter()
                        .map(|group| {
                            rebuild_node(
                                snapshot,
                                *group,
                                target_nodes,
                                panel_map,
                                node_map,
                                visiting,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    Node::Column(Column {
                        width: column.width,
                        mode: column.mode,
                        groups,
                        group_ratios: column.group_ratios,
                        last_expanded_width: column.last_expanded_width,
                    })
                }
            };
            visiting.remove(&target_id);

            let mapped = if let Some(current) = target_nodes.get_mut(target_id) {
                *current = rebuilt;
                target_id
            } else {
                target_nodes.insert(rebuilt)
            };
            node_map.insert(target_id, mapped);
            Ok(mapped)
        }

        let mut node_map = HashMap::<NodeId, NodeId>::new();
        let mut visiting = HashSet::<NodeId>::new();
        for old_id in snapshot.nodes.keys() {
            rebuild_node(
                snapshot,
                old_id,
                &mut candidate.nodes,
                &panel_map,
                &mut node_map,
                &mut visiting,
            )?;
        }

        let wanted_area_ids: HashSet<_> = snapshot.dock_areas.keys().collect();
        for id in candidate.dock_areas.keys().collect::<Vec<_>>() {
            if !wanted_area_ids.contains(&id) {
                candidate.dock_areas.remove(id);
            }
        }
        let mut area_map = HashMap::<DockAreaId, DockAreaId>::new();
        for (old_id, area) in snapshot.dock_areas.iter() {
            let root = node_map
                .get(&area.root)
                .copied()
                .ok_or(CommandError::MissingNode { node: area.root })?;
            let restored_area = DockArea { root, ..*area };
            let new_id = if let Some(current) = candidate.dock_areas.get_mut(old_id) {
                *current = restored_area;
                old_id
            } else {
                candidate.dock_areas.insert(restored_area)
            };
            area_map.insert(old_id, new_id);
        }

        candidate.main_areas = [
            match snapshot.main_areas[0] {
                Some(old_id) => {
                    Some(*area_map.get(&old_id).ok_or(CommandError::InvalidTarget {
                        reason: "history checkpoint has a missing left dock area",
                    })?)
                }
                None => None,
            },
            match snapshot.main_areas[1] {
                Some(old_id) => {
                    Some(*area_map.get(&old_id).ok_or(CommandError::InvalidTarget {
                        reason: "history checkpoint has a missing right dock area",
                    })?)
                }
                None => None,
            },
        ];

        let wanted_floating_ids: HashSet<_> = snapshot.floating_groups.keys().collect();
        for id in candidate.floating_groups.keys().collect::<Vec<_>>() {
            if !wanted_floating_ids.contains(&id) {
                candidate.floating_groups.remove(id);
            }
        }
        for (old_id, group) in snapshot.floating_groups.iter() {
            let area = area_map
                .get(&group.area)
                .copied()
                .ok_or(CommandError::InvalidTarget {
                    reason: "history checkpoint has a missing floating area",
                })?;
            let restored_group = FloatingGroup {
                area,
                bounds: group.bounds,
                z_order: group.z_order,
            };
            if let Some(current) = candidate.floating_groups.get_mut(old_id) {
                *current = restored_group;
            } else {
                candidate.floating_groups.insert(restored_group);
            }
        }

        candidate.last_dock_locations = snapshot.last_dock_locations.clone();
        candidate.last_dock_location_order = snapshot.last_dock_location_order.clone();
        candidate.canvas = snapshot.canvas;
        candidate.profile = snapshot.profile;
        candidate.chrome = snapshot.chrome;
        candidate.version = next_version;
        candidate
            .validate(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;
        *self = candidate;
        Ok(())
    }

    /// Opaque identity shared by Layout clones, but not separately loaded layouts.
    pub(crate) fn history_identity(&self) -> Weak<()> {
        Arc::downgrade(&self.history_identity)
    }
}
