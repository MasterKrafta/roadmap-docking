use super::*;
use crate::{AllowedArea, DragDropAction, DragDropRequest, DragHandle, DragPayload};

impl Layout {
    /// Resolves a completed drag request to either one command or an explicit
    /// no-op. It only inspects pure model state; no GPUI pointer/window types
    /// participate and the Layout is never mutated.
    pub fn resolve_drag_drop(
        &self,
        request: &DragDropRequest,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
        metrics.validate().map_err(|_| CommandError::InvalidValue {
            field: "metrics",
            reason: "dock metrics are invalid",
        })?;
        self.validate(metrics)
            .map_err(|errors| CommandError::InvalidLayout { errors })?;
        if !request.origin.is_finite() || !request.point.is_finite() {
            return Err(CommandError::InvalidValue {
                field: "pointer",
                reason: "drag pointer coordinates must be finite",
            });
        }
        if !request.handle.accepts(request.payload) {
            return Err(CommandError::InvalidTarget {
                reason: "drag handle does not match the payload kind",
            });
        }

        match (request.handle, request.payload) {
            (DragHandle::FloatingTitle, DragPayload::FloatingGroup { group }) => {
                if request.target.is_some() {
                    return Err(CommandError::InvalidTarget {
                        reason: "floating title drag moves the window and does not use a dock target",
                    });
                }
                let floating = self
                    .floating_group(group)
                    .copied()
                    .ok_or(CommandError::MissingFloatingGroup { group })?;
                let delta_x = request.point.x - request.origin.x;
                let delta_y = request.point.y - request.origin.y;
                let origin = floating
                    .bounds
                    .origin
                    .checked_offset(delta_x, delta_y)
                    .map_err(|_| CommandError::InvalidValue {
                        field: "origin",
                        reason: "floating title drag produced invalid window coordinates",
                    })?;
                if origin == floating.bounds.origin {
                    return Ok(DragDropAction::NoOp);
                }
                Ok(DragDropAction::Command(DockCommand::MoveFloating {
                    group,
                    origin,
                }))
            }
            (DragHandle::FloatingCapture, DragPayload::FloatingGroup { group }) => {
                let target = request.target.ok_or(CommandError::InvalidTarget {
                    reason: "floating capture drag requires a resolved dock/stack target",
                })?;
                if matches!(target, DropTarget::FreeArea) {
                    return Err(CommandError::InvalidTarget {
                        reason: "a floating capture drag requires a dock/stack target",
                    });
                }
                self.validate_drop_target(target, metrics)?;
                let root = self
                    .floating_group(group)
                    .and_then(|floating| self.dock_area(floating.area))
                    .map(|area| area.root)
                    .ok_or(CommandError::MissingFloatingGroup { group })?;
                if target
                    .referenced_node_id()
                    .is_some_and(|node| self.is_descendant_or_same(root, node))
                {
                    return Ok(DragDropAction::NoOp);
                }
                let moving_panels = self.panel_ids_in_subtree(root);
                self.validate_panel_ids_allowed_for_target(&moving_panels, target)?;
                Ok(DragDropAction::Command(DockCommand::DockFloating {
                    group,
                    target,
                }))
            }
            (DragHandle::PanelTab | DragHandle::FlyoutPanel, DragPayload::Panel { panel }) => {
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
            (DragHandle::GroupGrip, DragPayload::Group { group }) => {
                if !matches!(self.node(group), Some(Node::Tabs(_))) {
                    return Err(if self.node(group).is_some() {
                        CommandError::InvalidTarget {
                            reason: "group-grip drag source must be a Tabs node",
                        }
                    } else {
                        CommandError::MissingNode { node: group }
                    });
                }
                let target = request.target.ok_or(CommandError::InvalidTarget {
                    reason: "group-grip drag requires a resolved drop target",
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
                    self.validate_panel_ids_allowed_for_area(
                        &self.panel_ids_in_subtree(group),
                        AllowedArea::FLOATING,
                    )?;
                    return Ok(DragDropAction::Command(DockCommand::Float {
                        node: group,
                        bounds,
                    }));
                }
                self.validate_drop_target(target, metrics)?;
                self.validate_panel_ids_allowed_for_target(
                    &self.panel_ids_in_subtree(group),
                    target,
                )?;

                match target {
                    DropTarget::Tabs { .. } => Err(CommandError::InvalidTarget {
                        reason: "group grip carries all tabs; use GroupTabs to merge into a tab stack",
                    }),
                    DropTarget::GroupTabs {
                        group: target_group,
                        index,
                    } => {
                        if group == target_group {
                            Ok(DragDropAction::NoOp)
                        } else {
                            Ok(DragDropAction::Command(DockCommand::MergeGroup {
                                group,
                                target_group,
                                index,
                            }))
                        }
                    }
                    DropTarget::GroupEdge {
                        group: target_group,
                        zone,
                    } => {
                        if group == target_group
                            || self.group_edge_is_same_position(group, target_group, zone)
                        {
                            return Ok(DragDropAction::NoOp);
                        }
                        Ok(DragDropAction::Command(DockCommand::SplitGroup {
                            group,
                            target_group,
                            zone,
                            ratio: 0.5,
                        }))
                    }
                    DropTarget::MainColumn { side, index } => {
                        let target_columns = self.main_columns(side)?;
                        if let Some(source_column) = self.find_owner_column(group)
                            && let Some((source_side, source_index)) =
                                self.main_column_position(source_column)
                        {
                            let source_is_sole_group = matches!(
                                self.node(source_column),
                                Some(Node::Column(column)) if column.groups.len() == 1 && column.groups[0] == group
                            );
                            if source_side == side
                                && source_is_sole_group
                                && (index == source_index || index == source_index + 1)
                            {
                                return Ok(DragDropAction::NoOp);
                            }
                        }
                        if index > target_columns.len() {
                            return Err(CommandError::InvalidValue {
                                field: "index",
                                reason: "main-column insertion index is outside the dock",
                            });
                        }
                        Ok(DragDropAction::Command(
                            DockCommand::MoveGroupToMainColumn { group, side, index },
                        ))
                    }
                    DropTarget::ColumnSlot { column, index } => {
                        if self.group_slot_is_noop(group, column, index) {
                            return Ok(DragDropAction::NoOp);
                        }
                        Ok(DragDropAction::Command(DockCommand::MoveGroup {
                            group,
                            column,
                            index,
                        }))
                    }
                    DropTarget::RailSlot { column, index } => {
                        let group_index = self.rail_slot_group_index(column, index)?;
                        if self.group_slot_is_noop(group, column, group_index) {
                            return Ok(DragDropAction::NoOp);
                        }
                        Ok(DragDropAction::Command(DockCommand::MoveGroup {
                            group,
                            column,
                            index: group_index,
                        }))
                    }
                    DropTarget::FreeArea => Err(CommandError::InvalidTarget {
                        reason: "FreeArea must be converted to Float with explicit bounds",
                    }),
                }
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "drag handle and payload do not describe a supported drag operation",
            }),
        }
    }

    fn group_edge_is_same_position(
        &self,
        source_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
    ) -> bool {
        if source_group == target_group {
            return true;
        }
        let source_parent = self.find_parent(source_group);
        let target_parent = self.find_parent(target_group);
        if self.adjacent_group_edge_is_noop(
            source_group,
            target_group,
            zone,
            source_parent,
            target_parent,
        ) {
            return true;
        }
        if matches!(zone, DropZone::Left | DropZone::Right) {
            let Some(source_column) = self.find_owner_column(source_group) else {
                return false;
            };
            let Some(target_column) = self.find_owner_column(target_group) else {
                return false;
            };
            if source_column == target_column {
                return false;
            }
            let source_is_sole_group = matches!(
                self.node(source_column),
                Some(Node::Column(column)) if column.groups.len() == 1 && column.groups[0] == source_group
            );
            if !source_is_sole_group {
                return false;
            }
            let (Some((source_side, source_index)), Some((target_side, target_index))) = (
                self.main_column_position(source_column),
                self.main_column_position(target_column),
            ) else {
                return false;
            };
            return source_side == target_side
                && ((zone == DropZone::Left && source_index + 1 == target_index)
                    || (zone == DropZone::Right && target_index + 1 == source_index));
        }
        false
    }
}

