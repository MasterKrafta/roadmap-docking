use super::*;

impl Layout {
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
