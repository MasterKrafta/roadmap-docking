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
}
