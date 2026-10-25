use super::*;

impl Layout {
    pub(super) fn apply_panel_transfer_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
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
            _ => Err(CommandError::InvalidTarget {
                reason: "apply panel transfer command received an unsupported command",
            }),
        }
    }
}
