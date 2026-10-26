use super::*;

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
