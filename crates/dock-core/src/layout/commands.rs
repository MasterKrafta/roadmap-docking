use super::helpers::*;
use super::*;
use crate::AllowedArea;

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
            | DockCommand::MoveGroupToMainColumn { .. }
            | DockCommand::MergeGroup { .. }
            | DockCommand::SplitGroup { .. } => self.apply_group_command(command, metrics),
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
            DockCommand::CollapseGroup { group, collapsed } => {
                let old = match self.nodes.get(*group) {
                    Some(Node::Tabs(tabs)) => tabs.collapsed,
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "CollapseGroup target must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                if old == *collapsed {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Tabs(tabs)) = self.nodes.get_mut(*group) {
                    tabs.collapsed = *collapsed;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::CollapseGroup {
                        group: *group,
                        collapsed: old,
                    }),
                    persistent: true,
                })
            }
            DockCommand::SetColumnMode { column, mode } => {
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.clone(),
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "SetColumnMode target must be a Column node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if current.mode == *mode {
                    return Ok(CommandApplication::persistent());
                }
                let destination_area = if mode.is_rail() {
                    // Rail geometry represents each Tabs child as a stable panel
                    // entry, so reject malformed/non-tab columns before changing
                    // the mode instead of committing a layout that cannot render.
                    self.column_display_order(*column)?;
                    AllowedArea::ICON_RAIL
                } else {
                    AllowedArea::MAIN_DOCK
                };
                let moving_panels: Vec<_> = current
                    .groups
                    .iter()
                    .flat_map(|group| self.panel_ids_in_subtree(*group))
                    .collect();
                self.validate_panel_ids_allowed_for_area(&moving_panels, destination_area)?;
                let width = match *mode {
                    ColumnMode::IconsOnly => metrics.rail_width,
                    ColumnMode::IconsAndLabels => metrics.min_column_width,
                    ColumnMode::Expanded if current.mode.is_rail() => {
                        let minimum = self.minimum_expanded_column_width(*column, metrics)?;
                        let maximum = self
                            .maximum_expanded_column_width(*column)?
                            .unwrap_or(f32::MAX);
                        if maximum < minimum {
                            return Err(CommandError::InvalidValue {
                                field: "width",
                                reason: "expanded column minimum width exceeds its panel maximum",
                            });
                        }
                        current
                            .last_expanded_width
                            .unwrap_or(metrics.default_column_width)
                            .clamp(minimum, maximum)
                    }
                    ColumnMode::Expanded => current
                        .width
                        .max(self.minimum_column_width(*column, metrics)?),
                };
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.mode = *mode;
                    value.width = width;
                    value.last_expanded_width =
                        if current.mode == ColumnMode::Expanded && mode.is_rail() {
                            Some(current.width)
                        } else if *mode == ColumnMode::Expanded {
                            Some(width)
                        } else {
                            current.last_expanded_width
                        };
                }
                if !mode.is_rail() && self.flyout.is_some_and(|flyout| flyout.column == *column) {
                    self.flyout = None;
                }
                self.sync_main_column_ratios(*column)?;
                Ok(CommandApplication::persistent())
            }
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
            DockCommand::SetChrome { mode } => {
                let old = self.chrome;
                if old == *mode {
                    return Ok(CommandApplication::persistent());
                }
                self.chrome = *mode;
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::SetChrome { mode: old }),
                    persistent: true,
                })
            }
        }
    }
}
