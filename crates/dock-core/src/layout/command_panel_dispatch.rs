use super::*;
use super::helpers::geometry_error_reason;

impl Layout {
    pub(super) fn apply_panel_command(
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
            _ => Err(CommandError::InvalidTarget {
                reason: "apply panel command received an unsupported command",
            }),
        }
    }
}
