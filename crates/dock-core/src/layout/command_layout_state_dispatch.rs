use super::*;
use crate::AllowedArea;

impl Layout {
    pub(super) fn apply_layout_state_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
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
            _ => Err(CommandError::InvalidTarget {
                reason: "layout state dispatcher received an unsupported command",
            }),
        }
    }
}
