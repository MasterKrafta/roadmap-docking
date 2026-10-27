//! Commands for panels and their flyouts, including transfers and sizing.



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
            && let Some(active_index) = tabs.children.iter().position(|id| *id == active_node)
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

    pub(super) fn apply_panel_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MovePanelFromFlyout { .. }
            | DockCommand::FloatPanelFromFlyout { .. } => {
                self.apply_flyout_panel_transfer_command(command, metrics)
            }
            DockCommand::MovePanel { panel, target } => {
                self.apply_move_panel_command(panel, target, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "apply panel transfer command received an unsupported command",
            }),
        }
    }

    pub(super) fn apply_move_panel_command(
        &mut self,
        panel: &crate::PanelId,
        target: &DropTarget,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
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
            && self
                .tab_parent_index(panel_node)
                .is_some_and(|(source_group, _)| source_group == target_group)
        {
            // Dropping a panel on the edge of its own group is intentionally a
            // no-op. Use a different group's edge to detach the panel into a new group.
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
            && let Some(application) = self.try_reorder_panel_within_tabs(
                *panel,
                source_group,
                source_index,
                target_group,
                index,
            )?
        {
            return Ok(application);
        }
        let inverse = self.panel_transfer_inverse(*panel, panel_node);
        if let DropTarget::RailSlot { column, index } = *target {
            return self.move_panel_to_rail_slot(
                *panel,
                panel_node,
                column,
                index,
                metrics,
                inverse,
            );
        }
        self.move_panel_to_drop_target(panel_node, *target, metrics)?;
        Ok(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        })
    }

    pub(super) fn panel_transfer_inverse(
        &self,
        panel: crate::PanelId,
        panel_node: NodeId,
    ) -> Option<DockCommand> {
        self.tab_parent_index(panel_node)
            .and_then(|(group, index)| {
                let source_group_has_other_tabs = matches!(
                    self.nodes.get(group),
                    Some(Node::Tabs(tabs)) if tabs.children.len() > 1
                );
                source_group_has_other_tabs.then_some(DockCommand::MovePanel {
                    panel,
                    target: DropTarget::Tabs { group, index },
                })
            })
    }

    pub(super) fn move_panel_to_drop_target(
        &mut self,
        panel_node: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        self.remember_panel_dock_location(panel_node);
        self.detach_node(panel_node)?;
        self.insert_node_for_target(panel_node, target, metrics, Some(0.5))?;

        // In the Photoshop profile a center/tab drop is an explicit reveal action.
        // Same-group reorder/no-op paths return before reaching this helper.
        if self.profile == crate::Profile::Photoshop
            && let DropTarget::Tabs {
                group: target_group,
                ..
            } = target
            && let Some(Node::Tabs(tabs)) = self.nodes.get_mut(target_group)
        {
            tabs.collapsed = false;
        }

        self.remember_panel_dock_location(panel_node);
        Ok(())
    }

    pub(super) fn move_panel_to_rail_slot(
        &mut self,
        panel: crate::PanelId,
        panel_node: NodeId,
        column: NodeId,
        index: usize,
        metrics: &DockMetrics,
        inverse: Option<DockCommand>,
    ) -> Result<CommandApplication, CommandError> {
        let order = self.column_display_order(column)?;
        if let Some(source_index) = order.iter().position(|item| item.panel == panel)
            && (index == source_index || index == source_index + 1)
        {
            return Ok(CommandApplication::persistent());
        }

        self.remember_panel_dock_location(panel_node);
        let moved = self.insert_panel_for_rail_slot(panel_node, column, index, metrics)?;
        if !moved {
            return Ok(CommandApplication::persistent());
        }
        self.remember_panel_dock_location(panel_node);
        Ok(CommandApplication {
            actual_value: None,
            inverse,
            persistent: true,
        })
    }

    pub(super) fn apply_flyout_command(
        &mut self,
        command: &DockCommand,
    ) -> Result<CommandApplication, CommandError> {
        match command {
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
            _ => Err(CommandError::InvalidTarget {
                reason: "flyout dispatcher received an unsupported command",
            }),
        }
    }

    fn validate_flyout_panel_owner(
        &self,
        panel: crate::PanelId,
        reason: &'static str,
    ) -> Result<(), CommandError> {
        if self.flyout.is_some_and(|flyout| flyout.panel == panel) {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget { reason })
        }
    }

    fn finish_flyout_panel_transfer(
        &mut self,
        before: &Layout,
        mut applied: CommandApplication,
    ) -> CommandApplication {
        self.flyout = None;
        if self.same_persistent_state(before) {
            applied.persistent = false;
            applied.inverse = None;
        }
        applied
    }

    pub(super) fn apply_flyout_panel_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::MovePanelFromFlyout { panel, target } => {
                self.validate_flyout_panel_owner(
                    *panel,
                    "MovePanelFromFlyout requires the dragged panel to own the active flyout",
                )?;

                let before = self.clone();
                let applied = self.apply_command(
                    &DockCommand::MovePanel {
                        panel: *panel,
                        target: *target,
                    },
                    metrics,
                )?;
                // A valid release ends the flyout only after the nested command succeeds.
                Ok(self.finish_flyout_panel_transfer(&before, applied))
            }
            DockCommand::FloatPanelFromFlyout { panel, bounds } => {
                self.validate_flyout_panel_owner(
                    *panel,
                    "FloatPanelFromFlyout requires the dragged panel to own the active flyout",
                )?;
                let node = self
                    .find_panel_node(*panel)
                    .ok_or(CommandError::MissingPanel { panel: *panel })?;
                let before = self.clone();
                let applied = self.apply_command(
                    &DockCommand::Float {
                        node,
                        bounds: *bounds,
                    },
                    metrics,
                )?;
                Ok(self.finish_flyout_panel_transfer(&before, applied))
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "apply flyout panel transfer command received an unsupported command",
            }),
        }
    }

}

