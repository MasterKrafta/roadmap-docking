use super::*;

impl Layout {
    /// Applies one command transactionally. All edits happen on a candidate copy;
    /// the live layout is replaced only after normalization and validation succeed.
    pub fn apply(
        &mut self,
        command: DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandOutcome, CommandError> {
        tracing::debug!(?command, version = self.version, "applying layout command");
        let prior_version = self.version;
        let result = self.apply_transaction(&command, metrics);
        match &result {
            Ok(outcome) if outcome.new_version != prior_version => {
                tracing::info!(
                    ?command,
                    old_version = prior_version,
                    new_version = outcome.new_version,
                    "layout command committed"
                );
            }
            Ok(_) => {
                tracing::debug!(
                    ?command,
                    version = prior_version,
                    "layout command accepted without version change"
                );
            }
            Err(error) => {
                tracing::warn!(
                    ?command,
                    ?error,
                    version = prior_version,
                    "layout command rejected"
                );
            }
        }
        result
    }

    pub(super) fn apply_transaction(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandOutcome, CommandError> {
        self.validate_cached(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;

        let mut candidate = self.clone();
        let applied = candidate.apply_command(command, metrics)?;
        candidate
            .normalize_in_place(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;
        candidate.validation_cache.set(Some((candidate.version, *metrics)));

        if candidate.same_state(self) {
            return Ok(CommandOutcome {
                changed_nodes: Vec::new(),
                new_version: self.version,
                actual_value: applied.actual_value,
                inverse: None,
            });
        }

        if !applied.persistent {
            candidate.version = self.version;
            *self = candidate;
            return Ok(CommandOutcome {
                changed_nodes: Vec::new(),
                new_version: self.version,
                actual_value: applied.actual_value,
                inverse: None,
            });
        }

        let next_version = self
            .version
            .checked_add(1)
            .ok_or(CommandError::InvalidValue {
                field: "version",
                reason: "layout version overflow",
            })?;
        let changed_nodes = self.changed_node_ids(&candidate, command);
        candidate.version = next_version;
        candidate.validation_cache.set(Some((next_version, *metrics)));
        *self = candidate;

        Ok(CommandOutcome {
            changed_nodes,
            new_version: next_version,
            actual_value: applied.actual_value,
            inverse: applied.inverse,
        })
    }

    pub(super) fn apply_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::Float { .. }
            | DockCommand::FloatGroupAsColumn { .. }
            | DockCommand::FloatPanelAsGroup { .. }
            | DockCommand::MoveFloating { .. }
            | DockCommand::ResizeFloating { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::RaiseFloating { .. }
            | DockCommand::DockFloating { .. } => self.apply_floating_command(command, metrics),
            DockCommand::OpenPanel { .. }
            | DockCommand::ClosePanel { .. }
            | DockCommand::CloseGroup { .. }
            | DockCommand::ActivateTab { .. }
            | DockCommand::ReorderTab { .. } => self.apply_panel_command(command, metrics),
            DockCommand::MovePanelFromFlyout { .. }
            | DockCommand::FloatPanelFromFlyout { .. }
            | DockCommand::MovePanel { .. } => self.apply_panel_transfer_command(command, metrics),
            DockCommand::MoveGroup { .. }
            | DockCommand::MoveGroupToMainColumn { .. } => {
                self.apply_group_transfer_command(command, metrics)
            }
            DockCommand::MergeGroup { .. } | DockCommand::SplitGroup { .. } => {
                self.apply_group_command(command, metrics)
            },
            DockCommand::ResizeGroups { .. }
            | DockCommand::ResetColumnRatios { .. }
            | DockCommand::RestoreGroupRatios { .. } => {
                self.apply_group_resize_command(command, metrics)
            }
            DockCommand::ResizeColumn { .. }
            | DockCommand::ResizeColumnBoundary { .. }
            | DockCommand::ResizeSplitColumnBoundary { .. }
            | DockCommand::ResetMainColumnRatios { .. } => {
                self.apply_column_resize_command(command, metrics)
            }
            DockCommand::CollapseGroup { .. }
            | DockCommand::SetColumnMode { .. }
            | DockCommand::SetChrome { .. } => self.apply_layout_state_command(command, metrics),
            DockCommand::OpenFlyout { .. }
            | DockCommand::CloseFlyout
            | DockCommand::DismissFlyout
            | DockCommand::SetFlyoutAutoCollapse { .. }
            | DockCommand::ResizeFlyout { .. } => self.apply_flyout_command(command),
        }
    }

pub(super) fn same_state(&self, other: &Self) -> bool {
        self.same_persistent_state(other)
            && self.flyout == other.flyout
            && self.flyout_auto_collapse == other.flyout_auto_collapse
            && self.flyout_widths == other.flyout_widths
            && self.flyout_width_order == other.flyout_width_order
    }

    pub(super) fn same_persistent_state(&self, other: &Self) -> bool {
        self.nodes.iter().eq(other.nodes.iter())
            && self.panel_instances.iter().eq(other.panel_instances.iter())
            && self.dock_areas.iter().eq(other.dock_areas.iter())
            && self.main_areas == other.main_areas
            && self.floating_groups.iter().eq(other.floating_groups.iter())
            && self.last_dock_locations == other.last_dock_locations
            && self.canvas == other.canvas
            && self.profile == other.profile
            && self.version == other.version
            && self.chrome == other.chrome
    }

    pub(super) fn changed_node_ids(&self, other: &Self, command: &DockCommand) -> Vec<NodeId> {
        let mut changed = Vec::new();
        for (id, node) in self.nodes.iter() {
            if other.nodes.get(id) != Some(node) {
                changed.push(id);
            }
        }
        for (id, _) in other.nodes.iter() {
            if !self.nodes.contains_key(id) {
                changed.push(id);
            }
        }

        // Some persistent presentation commands change geometry/stacking without
        // changing the underlying Node values; return roots for adapter invalidation.
        let mut affected = Vec::new();
        match command {
            DockCommand::MoveFloating { group, .. }
            | DockCommand::ResizeFloatingBounds { group, .. }
            | DockCommand::ResizeFloating { group, .. }
            | DockCommand::RaiseFloating { group }
            | DockCommand::DockFloating { group, .. } => {
                for layout in [self, other] {
                    if let Some(floating) = layout.floating_group(*group)
                        && let Some(area) = layout.dock_area(floating.area)
                    {
                        affected.push(area.root);
                    }
                }
            }
            DockCommand::SetChrome { .. } => {
                for layout in [self, other] {
                    affected.extend(layout.main_areas.iter().flatten().filter_map(|area_id| layout.dock_area(*area_id).map(|area| area.root)));
                    affected.extend(layout.floating_groups.values().filter_map(|group| layout.dock_area(group.area).map(|area| area.root)));
                }
            }
            _ => {}
        }
        for node in affected {
            if !changed.contains(&node) {
                changed.push(node);
            }
        }
        changed
    }
}
