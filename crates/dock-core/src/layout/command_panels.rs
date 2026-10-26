use super::*;
use crate::PanelConstraints;
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
            } => self.apply_open_panel_command(key, constraints, target, metrics),
            DockCommand::ClosePanel { panel } => self.apply_close_panel_command(panel),
            DockCommand::CloseGroup { group } => self.apply_close_group_command(group),
            DockCommand::ActivateTab { group, panel } => {
                self.apply_activate_tab_command(group, panel)
            }
            DockCommand::ReorderTab {
                group,
                panel,
                to_index,
            } => self.apply_reorder_tab_command(group, panel, to_index),
            _ => Err(CommandError::InvalidTarget {
                reason: "apply panel command received an unsupported command",
            }),
        }
    }

    pub(super) fn apply_open_panel_command(
        &mut self,
        key: &PanelKey,
        constraints: &PanelConstraints,
        target: &Option<DropTarget>,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
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

        let resolved_target = self.resolve_and_validate_open_panel_target(
            key,
            *target,
            *constraints,
            metrics,
        )?;
        self.insert_new_panel_for_open_target(
            key,
            *constraints,
            resolved_target,
            metrics,
        )
    }

    pub(super) fn resolve_and_validate_open_panel_target(
        &self,
        key: &PanelKey,
        target: Option<DropTarget>,
        constraints: PanelConstraints,
        metrics: &DockMetrics,
    ) -> Result<DropTarget, CommandError> {
        let resolved_target = match target {
            Some(target) => target,
            None => self.resolve_open_target(key, metrics)?,
        };
        self.validate_drop_target(resolved_target, metrics)?;
        self.validate_constraints_allowed_for_target(constraints, resolved_target)?;
        Ok(resolved_target)
    }

    pub(super) fn insert_new_panel_for_open_target(
        &mut self,
        key: &PanelKey,
        constraints: PanelConstraints,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        let panel_id = self
            .panel_instances
            .insert(PanelInstance::new(key.clone(), constraints));
        let panel_node = self.nodes.insert(Node::Panel(crate::Panel::new(panel_id)));
        if let DropTarget::RailSlot { column, index } = target {
            self.insert_panel_for_rail_slot(panel_node, column, index, metrics)?;
        } else {
            self.insert_node_for_target(panel_node, target, metrics, Some(0.5))?;
            if self.profile == Profile::Photoshop
                && let DropTarget::Tabs { group, .. } = target
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

    pub(super) fn try_reopen_existing_singleton_panel(
        &mut self,
        key: &PanelKey,
        requested_singleton: bool,
    ) -> Result<Option<CommandApplication>, CommandError> {
        let Some((existing_id, _)) = self.panel_instances.iter().find(|(_, instance)| {
            instance.key == *key && (requested_singleton || instance.constraints.singleton)
        }) else {
            return Ok(None);
        };

        let panel_node = self
            .find_panel_node(existing_id)
            .ok_or(CommandError::MissingPanel { panel: existing_id })?;
        let Some((group, index)) = self.tab_parent_index(panel_node) else {
            return Ok(Some(CommandApplication::persistent()));
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
            return Ok(Some(CommandApplication {
                actual_value: None,
                inverse: previous_active
                    .map(|panel| DockCommand::ActivateTab { group, panel }),
                persistent: true,
            }));
        }
        if self.flyout != previous_flyout {
            return Ok(Some(CommandApplication::transient()));
        }
        Ok(Some(CommandApplication::persistent()))
    }

    pub(super) fn apply_close_panel_command(
        &mut self,
        panel: &crate::PanelId,
    ) -> Result<CommandApplication, CommandError> {
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

    pub(super) fn apply_close_group_command(
        &mut self,
        group: &NodeId,
    ) -> Result<CommandApplication, CommandError> {
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

    pub(super) fn apply_activate_tab_command(
        &mut self,
        group: &NodeId,
        panel: &crate::PanelId,
    ) -> Result<CommandApplication, CommandError> {
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
        let previous = tabs
            .children
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

    pub(super) fn apply_reorder_tab_command(
        &mut self,
        group: &NodeId,
        panel: &crate::PanelId,
        to_index: &usize,
    ) -> Result<CommandApplication, CommandError> {
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

    pub(super) fn try_reorder_panel_within_tabs(
        &mut self,
        panel: crate::PanelId,
        source_group: NodeId,
        source_index: usize,
        target_group: NodeId,
        target_index: usize,
    ) -> Result<Option<CommandApplication>, CommandError> {
        if source_group != target_group {
            return Ok(None);
        }
        if target_index == source_index || target_index == source_index + 1 {
            return Ok(Some(CommandApplication::persistent()));
        }

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
        let insert_index = if target_index > source_index {
            target_index - 1
        } else {
            target_index
        };
        tabs.children
            .insert(insert_index.min(tabs.children.len()), moved);
        if let Some(active_node) = active_node
            && let Some(active_index) = tabs.children.iter().position(|id| id == active_node)
        {
            tabs.active_index = active_index;
        }

        Ok(Some(CommandApplication {
            actual_value: None,
            inverse: Some(DockCommand::ReorderTab {
                group: source_group,
                panel,
                to_index: source_index,
            }),
            persistent: true,
        }))
    }
}
