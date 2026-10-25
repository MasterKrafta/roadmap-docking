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

                if let Some(application) =
                    self.try_reopen_existing_singleton_panel(key, constraints.singleton)?
                {
                    return Ok(application);
                }

                let resolved_target = match *target {
                    Some(target) => target,
                    None => self.resolve_open_target(key, metrics)?,
                };
                self.validate_drop_target(resolved_target, metrics)?;
                self.validate_constraints_allowed_for_target(*constraints, resolved_target)?;
                self.insert_new_panel_for_open_target(
                    key,
                    *constraints,
                    resolved_target,
                    metrics,
                )
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
