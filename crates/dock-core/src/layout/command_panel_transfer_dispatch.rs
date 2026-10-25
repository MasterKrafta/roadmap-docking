use super::*;

impl Layout {
    fn move_panel_to_drop_target(
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

    fn move_panel_to_rail_slot(
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
            _ => Err(CommandError::InvalidTarget {
                reason: "apply panel transfer command received an unsupported command",
            }),
        }
    }
}
