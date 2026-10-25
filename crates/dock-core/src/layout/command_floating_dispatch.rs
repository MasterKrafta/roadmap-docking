use super::*;
use super::helpers::geometry_error_reason;
use crate::AllowedArea;

impl Layout {
    pub(super) fn apply_floating_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::Float { node, bounds } => {
                // Bounds use model-space logical pixels. Visible-screen clamping
                // belongs to the geometry/render adapter, not to Core/GPUI.
                if !self.nodes.contains_key(*node) {
                    return Err(CommandError::MissingNode { node: *node });
                }
                bounds
                    .validate_non_empty()
                    .map_err(|error| CommandError::InvalidValue {
                        field: "bounds",
                        reason: geometry_error_reason(error),
                    })?;
                let moving_panels = self.panel_ids_in_subtree(*node);
                self.validate_panel_ids_allowed_for_area(&moving_panels, AllowedArea::FLOATING)?;
                let supports_photoshop_floating_root = match self.nodes.get(*node) {
                    Some(Node::Panel(_) | Node::Tabs(_)) => true,
                    Some(Node::Column(column)) => column
                        .groups
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Tabs(_)))),
                    Some(Node::Split(split)) if split.axis == Axis::X => split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))),
                    _ => false,
                };
                if self.profile == Profile::Photoshop && !supports_photoshop_floating_root {
                    return Err(CommandError::UnsupportedForProfile {
                        profile: self.profile,
                        reason: "only a panel, tab group, column, or horizontal composition of columns can become a floating root",
                    });
                }
                let next_z = self.next_floating_z_order()?;
                let panel_root = matches!(self.nodes.get(*node), Some(Node::Panel(_)));
                self.remember_subtree_dock_locations(*node);
                self.detach_node(*node)?;
                let root = if panel_root && self.profile == Profile::Photoshop {
                    self.nodes.insert(Node::Tabs(crate::Tabs {
                        children: vec![*node],
                        active_index: 0,
                        collapsed: false,
                    }))
                } else {
                    *node
                };
                let area_id = self.dock_areas.insert(DockArea::floating(root));
                self.floating_groups
                    .insert(FloatingGroup::new(area_id, *bounds, next_z));
                Ok(CommandApplication::persistent())
            }
            DockCommand::FloatGroupAsColumn { group, bounds } => {
                self.float_group_as_column(*group, *bounds, metrics)
            }
            DockCommand::FloatPanelAsGroup { panel, bounds } => {
                self.float_panel_as_group(*panel, *bounds, metrics)
            }
            DockCommand::MoveFloating { .. }
            | DockCommand::ResizeFloating { .. }
            | DockCommand::ResizeFloatingBounds { .. }
            | DockCommand::RaiseFloating { .. } => {
                self.apply_floating_window_command(command, metrics)
            }
            DockCommand::DockFloating { group, target } => {
                self.apply_dock_floating_command(group, target, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "floating command dispatcher received a non-floating command",
            }),
        }
    }
}
