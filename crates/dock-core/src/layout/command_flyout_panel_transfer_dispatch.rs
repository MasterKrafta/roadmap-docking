use super::*;

impl Layout {
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
                let mut applied = self.apply_command(
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
                let mut applied = self.apply_command(
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
