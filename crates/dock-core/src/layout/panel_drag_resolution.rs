use super::*;
use crate::{AllowedArea, DragDropAction, DragDropRequest, DragHandle, PanelId};

impl Layout {
    pub(super) fn resolve_panel_drag(
        &self,
        request: &DragDropRequest,
        panel: PanelId,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
        if request.handle == DragHandle::FlyoutPanel
            && !self.flyout().is_some_and(|flyout| flyout.panel == panel)
        {
            return Err(CommandError::InvalidTarget {
                reason: "flyout panel drag must start from the currently open flyout",
            });
        }
        if self.panel_instance(panel).is_none() {
            return Err(CommandError::MissingPanel { panel });
        }
        let panel_node = self
            .find_panel_node(panel)
            .ok_or(CommandError::MissingPanel { panel })?;
        let target = request.target.ok_or(CommandError::InvalidTarget {
            reason: "panel-tab drag requires a resolved drop target",
        })?;
        if matches!(target, DropTarget::FreeArea) {
            let bounds = request.floating_bounds.ok_or(CommandError::InvalidTarget {
                reason: "FreeArea Float requests require explicit window bounds",
            })?;
            bounds
                .validate_non_empty()
                .map_err(|_| CommandError::InvalidValue {
                    field: "bounds",
                    reason: "floating bounds must be finite and positive",
                })?;
            self.validate_panel_ids_allowed_for_area(&[panel], AllowedArea::FLOATING)?;
            let command = if request.handle == DragHandle::FlyoutPanel {
                DockCommand::FloatPanelFromFlyout { panel, bounds }
            } else {
                DockCommand::Float {
                    node: panel_node,
                    bounds,
                }
            };
            return Ok(DragDropAction::Command(command));
        }
        self.validate_drop_target(target, metrics)?;

        if let (Some((source_group, source_index)), DropTarget::Tabs { group, index }) =
            (self.tab_parent_index(panel_node), target)
            && source_group == group
            && (index == source_index || index == source_index + 1)
        {
            return Ok(if request.handle == DragHandle::FlyoutPanel {
                DragDropAction::Command(DockCommand::MovePanelFromFlyout { panel, target })
            } else {
                DragDropAction::NoOp
            });
        }
        if let Some((source_group, _)) = self.tab_parent_index(panel_node) {
            let source_group_has_only_panel = matches!(
                self.node(source_group),
                Some(Node::Tabs(tabs)) if tabs.children.len() == 1
            );
            if source_group_has_only_panel
                && let Some(source_column) = self.find_owner_column(source_group)
            {
                let source_column_has_only_group = matches!(
                    self.node(source_column),
                    Some(Node::Column(column)) if column.groups.len() == 1 && column.groups[0] == source_group
                );
                if source_column_has_only_group
                    && let Some((source_side, source_index)) =
                        self.main_column_position(source_column)
                {
                    match target {
                        DropTarget::MainColumn { side, index }
                            if side == source_side
                                && (index == source_index || index == source_index + 1) =>
                        {
                            return Ok(if request.handle == DragHandle::FlyoutPanel {
                                DragDropAction::Command(DockCommand::MovePanelFromFlyout {
                                    panel,
                                    target,
                                })
                            } else {
                                DragDropAction::NoOp
                            });
                        }
                        DropTarget::ColumnSlot { column, index }
                        | DropTarget::RailSlot { column, index }
                            if column == source_column
                                && self.node(column).is_some_and(|node| {
                                    matches!(
                                        node,
                                        Node::Column(value) if value.groups.len() == 1
                                            && (index == 0 || index == 1)
                                    )
                                }) =>
                        {
                            return Ok(if request.handle == DragHandle::FlyoutPanel {
                                DragDropAction::Command(DockCommand::MovePanelFromFlyout {
                                    panel,
                                    target,
                                })
                            } else {
                                DragDropAction::NoOp
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
        // Moving a tab to its own group's edge can be a valid split:
        // the panel leaves its Tabs node and becomes a new adjacent group.
        // Only an insertion at the current tab slot is a no-op.

        // A panel tab remains a panel payload; GroupTabs is reserved for
        // a group-grip gesture and may never absorb only one dragged tab.
        if matches!(target, DropTarget::GroupTabs { .. }) {
            return Err(CommandError::InvalidTarget {
                reason: "panel-tab drag cannot merge a whole group; use a Tabs insertion target",
            });
        }
        self.validate_panel_ids_allowed_for_target(&[panel], target)?;
        let command = if request.handle == DragHandle::FlyoutPanel {
            DockCommand::MovePanelFromFlyout { panel, target }
        } else {
            DockCommand::MovePanel { panel, target }
        };
        Ok(DragDropAction::Command(command))
    }
}
