//! Drag-and-drop target validation and request resolution.

use super::*;
use crate::{
    AllowedArea, DragDropAction, DragDropRequest, DragHandle, DragPayload, FloatingGroupId,
    NodeId, PanelId,
};

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
                self.resolve_floating_title_drag(request, group)
            }
            (DragHandle::FloatingCapture, DragPayload::FloatingGroup { group }) => {
                self.resolve_floating_capture_drag(request, group, metrics)
            }
            (DragHandle::PanelTab | DragHandle::FlyoutPanel, DragPayload::Panel { panel }) => {
                self.resolve_panel_drag(request, panel, metrics)
            }
            (DragHandle::GroupGrip, DragPayload::Group { group }) => {
                self.resolve_group_grip_drag(request, group, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "drag handle and payload do not describe a supported drag operation",
            }),
        }
    }

    pub(super) fn resolve_floating_title_drag(
        &self,
        request: &DragDropRequest,
        group: FloatingGroupId,
    ) -> Result<DragDropAction, CommandError> {
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

    pub(super) fn resolve_floating_capture_drag(
        &self,
        request: &DragDropRequest,
        group: FloatingGroupId,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
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
        // A multi-tab group can be split by dropping one of its tabs on
        // its own edge. A single-tab group has nothing to split, so make that
        // gesture a true NoOp (a flyout release still needs to close the flyout).
        if let (Some((source_group, _)), DropTarget::GroupEdge { group: target_group, .. }) =
            (self.tab_parent_index(panel_node), target)
            && source_group == target_group
            && matches!(
                self.node(source_group),
                Some(Node::Tabs(tabs)) if tabs.children.len() <= 1
            )
        {
            return Ok(if request.handle == DragHandle::FlyoutPanel {
                DragDropAction::Command(DockCommand::MovePanelFromFlyout { panel, target })
            } else {
                DragDropAction::NoOp
            });
        }

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

pub(super) fn resolve_group_grip_drag(
        &self,
        request: &DragDropRequest,
        group: NodeId,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
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

    /// Shared no-op contract for dropping a group at the edge of an adjacent
    /// sibling under the same Column or vertical Split.
    ///
    /// Callers supply parent IDs so indexed callers can avoid rescanning the
    /// whole arena; both offer generation and drag resolution use this rule.
    pub(crate) fn adjacent_group_edge_is_noop(
        &self,
        source_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        source_parent: Option<NodeId>,
        target_parent: Option<NodeId>,
    ) -> bool {
        if !matches!(zone, DropZone::Top | DropZone::Bottom) {
            return false;
        }
        let (Some(parent), Some(target_parent)) = (source_parent, target_parent) else {
            return false;
        };
        if parent != target_parent {
            return false;
        }
        let siblings = match self.nodes.get(parent) {
            Some(Node::Column(column)) => &column.groups,
            Some(Node::Split(split)) if split.axis == Axis::Y => &split.children,
            _ => return false,
        };
        let (Some(source_index), Some(target_index)) = (
            siblings.iter().position(|id| *id == source_group),
            siblings.iter().position(|id| *id == target_group),
        ) else {
            return false;
        };
        (zone == DropZone::Top && source_index + 1 == target_index)
            || (zone == DropZone::Bottom && target_index + 1 == source_index)
    }

    /// Returns whether a group insertion slot leaves the group in the same
    /// place. The index is measured after removing the source group, matching
    /// the command and drop-zone insertion-slot contract.
    pub(crate) fn group_slot_is_noop(
        &self,
        group: NodeId,
        column_id: NodeId,
        index: usize,
    ) -> bool {
        // Preserve the ownership check: a malformed tree may reference the same
        // group from multiple columns. Do not treat a slot in a non-owner column
        // as a no-op merely because its group list happens to contain that ID.
        if self.find_owner_column(group) != Some(column_id) {
            return false;
        }
        let Some(Node::Column(column)) = self.nodes.get(column_id) else {
            return false;
        };
        let Some(source_index) = column.groups.iter().position(|id| *id == group) else {
            return false;
        };
        index == source_index || index == source_index + 1
    }
}

impl Layout {
    pub(super) fn allowed_area_for_column_target(
        &self,
        column_id: NodeId,
    ) -> Result<AllowedArea, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "group destination must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: column_id }),
        };

        if self.find_floating_root_containing(column_id).is_some() {
            return Ok(AllowedArea::FLOATING);
        }

        for side in [DockSide::Left, DockSide::Right] {
            if self
                .main_area(side)
                .is_some_and(|area| self.is_descendant_or_same(area.root, column_id))
            {
                return Ok(if column.mode.is_rail() {
                    AllowedArea::ICON_RAIL
                } else {
                    AllowedArea::MAIN_DOCK
                });
            }
        }

        Err(CommandError::InvalidTarget {
            reason: "group destination column is not attached to a main or floating dock area",
        })
    }

    pub(super) fn allowed_area_for_group_target(
        &self,
        group: NodeId,
    ) -> Result<AllowedArea, CommandError> {
        if !matches!(self.nodes.get(group), Some(Node::Tabs(_))) {
            return Err(if self.nodes.contains_key(group) {
                CommandError::InvalidTarget {
                    reason: "drop group target must be a Tabs node",
                }
            } else {
                CommandError::MissingNode { node: group }
            });
        }

        // Floating membership takes precedence because Generic layouts may contain
        // arbitrary nested Split trees inside a floating area.
        if self.find_floating_root_containing(group).is_some() {
            return Ok(AllowedArea::FLOATING);
        }

        if let Some(column_id) = self.find_owner_column(group)
            && self.main_column_position(column_id).is_some()
        {
            return match self.nodes.get(column_id) {
                Some(Node::Column(column)) if column.mode.is_rail() => {
                    Ok(AllowedArea::ICON_RAIL)
                }
                Some(Node::Column(_)) => Ok(AllowedArea::MAIN_DOCK),
                _ => Err(CommandError::InvalidTarget {
                    reason: "drop group owner is not a Column",
                }),
            };
        }

        for side in [DockSide::Left, DockSide::Right] {
            if self
                .main_area(side)
                .is_some_and(|area| self.is_descendant_or_same(area.root, group))
            {
                return Ok(AllowedArea::MAIN_DOCK);
            }
        }

        Err(CommandError::InvalidTarget {
            reason: "drop target is not attached to a main or floating dock area",
        })
    }

    pub(super) fn allowed_area_for_target(
        &self,
        target: DropTarget,
    ) -> Result<AllowedArea, CommandError> {
        match target {
            DropTarget::Tabs { group, .. }
            | DropTarget::GroupTabs { group, .. }
            | DropTarget::GroupEdge { group, .. } => self.allowed_area_for_group_target(group),
            DropTarget::MainColumn { .. } | DropTarget::ColumnSlot { .. } => {
                Ok(AllowedArea::MAIN_DOCK)
            }
            DropTarget::RailSlot { .. } => Ok(AllowedArea::ICON_RAIL),
            DropTarget::FreeArea => Ok(AllowedArea::FLOATING),
        }
    }

    pub(super) fn validate_panel_ids_allowed_for_area(
        &self,
        panels: &[PanelId],
        area: AllowedArea,
    ) -> Result<(), CommandError> {
        for panel in panels {
            let instance = self
                .panel_instance(*panel)
                .ok_or(CommandError::MissingPanel { panel: *panel })?;
            if !instance.constraints.allows_area(area) {
                return Err(CommandError::InvalidTarget {
                    reason: allowed_area_rejection_reason(area),
                });
            }
        }
        Ok(())
    }

    pub(super) fn validate_constraints_allowed_for_target(
        &self,
        constraints: crate::PanelConstraints,
        target: DropTarget,
    ) -> Result<(), CommandError> {
        let area = self.allowed_area_for_target(target)?;
        if constraints.allows_area(area) {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget {
                reason: allowed_area_rejection_reason(area),
            })
        }
    }

    pub(super) fn validate_panel_ids_allowed_for_target(
        &self,
        panels: &[PanelId],
        target: DropTarget,
    ) -> Result<(), CommandError> {
        let area = self.allowed_area_for_target(target)?;
        self.validate_panel_ids_allowed_for_area(panels, area)
    }

    pub(super) fn validate_drop_target(
        &self,
        target: DropTarget,
        _metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        match target {
            DropTarget::Tabs { group, index } => match self.nodes.get(group) {
                Some(Node::Tabs(_)) if self.is_group_in_icon_rail(group) => {
                    Err(CommandError::InvalidTarget {
                        reason: "tab insertion target is an icon rail slot; use RailSlot",
                    })
                }
                Some(Node::Tabs(tabs)) if index <= tabs.children.len() => Ok(()),
                Some(Node::Tabs(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "tab insertion index is outside the group",
                }),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "tab target must be a Tabs node",
                }),
                None => Err(CommandError::MissingNode { node: group }),
            },
            DropTarget::GroupTabs { group, index } => match self.nodes.get(group) {
                Some(Node::Tabs(_)) if self.is_group_in_icon_rail(group) => {
                    Err(CommandError::InvalidTarget {
                        reason: "group-merge target is an icon rail slot; use RailSlot",
                    })
                }
                Some(Node::Tabs(tabs)) if index <= tabs.children.len() => Ok(()),
                Some(Node::Tabs(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "group-tab insertion index is outside the group",
                }),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "group-tab target must be a Tabs node",
                }),
                None => Err(CommandError::MissingNode { node: group }),
            },
            DropTarget::GroupEdge { group, zone } => {
                if !matches!(
                    zone,
                    DropZone::Left | DropZone::Right | DropZone::Top | DropZone::Bottom
                ) {
                    return Err(CommandError::InvalidTarget {
                        reason: "group-edge target requires a cardinal edge zone",
                    });
                }
                if self.is_group_in_icon_rail(group) {
                    return Err(CommandError::InvalidTarget {
                        reason: "group-edge target is an icon rail slot; use RailSlot",
                    });
                }
                match self.nodes.get(group) {
                    Some(Node::Tabs(_)) => {}
                    Some(_) => {
                        return Err(CommandError::InvalidTarget {
                            reason: "group-edge target must be a Tabs node",
                        });
                    }
                    None => return Err(CommandError::MissingNode { node: group }),
                }
                if self.profile == Profile::Photoshop {
                    match zone {
                        DropZone::Left | DropZone::Right => {
                            let Some(column) = self.find_owner_column(group) else {
                                return Err(CommandError::InvalidTarget {
                                    reason: if self.find_floating_root_containing(group).is_some() {
                                        "horizontal splits are not supported inside a floating stack"
                                    } else {
                                        "horizontal group splits require a main-dock column"
                                    },
                                });
                            };
                            if self.main_column_position(column).is_none() {
                                return Err(CommandError::InvalidTarget {
                                    reason: "horizontal group splits require a column in a main dock",
                                });
                            }
                        }
                        DropZone::Top | DropZone::Bottom => {
                            if self.find_owner_column(group).is_none()
                                && self.find_floating_root_containing(group).is_none()
                            {
                                return Err(CommandError::InvalidTarget {
                                    reason: "vertical group splits require a column or floating stack",
                                });
                            }
                        }
                        _ => {
                            return Err(CommandError::InvalidTarget {
                                reason: "group-edge target requires a cardinal edge zone",
                            });
                        }
                    }
                }
                Ok(())
            }
            DropTarget::MainColumn { side, index } => {
                let columns = self.main_columns(side)?;
                if index <= columns.len() {
                    Ok(())
                } else {
                    Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "main column insertion index is outside the dock",
                    })
                }
            }
            DropTarget::ColumnSlot { column, index } => match self.nodes.get(column) {
                Some(Node::Column(value)) if value.mode.is_rail() => {
                    Err(CommandError::InvalidTarget {
                        reason: "expanded-column insertion target cannot be a rail; use RailSlot",
                    })
                }
                Some(Node::Column(value)) if self.main_column_position(column).is_none() => {
                    let _ = value;
                    Err(CommandError::InvalidTarget {
                        reason: "column insertion target must belong to a main dock",
                    })
                }
                Some(Node::Column(value)) if index <= value.groups.len() => Ok(()),
                Some(Node::Column(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "column group insertion index is outside the column",
                }),
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "column slot target must be a Column node",
                }),
                None => Err(CommandError::MissingNode { node: column }),
            },
            DropTarget::FreeArea => Err(CommandError::InvalidTarget {
                reason: "FreeArea must be resolved by Float with explicit window bounds",
            }),
            DropTarget::RailSlot { column, index } => match self.nodes.get(column) {
                Some(Node::Column(value)) if !value.mode.is_rail() => {
                    Err(CommandError::InvalidTarget {
                        reason: "rail insertion target must use a rail mode",
                    })
                }
                Some(Node::Column(_)) if self.main_column_position(column).is_none() => {
                    Err(CommandError::InvalidTarget {
                        reason: "rail insertion target must belong to a main dock column",
                    })
                }
                Some(Node::Column(_)) => {
                    let count = self.column_display_order(column)?.len();
                    if index <= count {
                        Ok(())
                    } else {
                        Err(CommandError::InvalidValue {
                            field: "index",
                            reason: "rail insertion index is outside the flattened icon order",
                        })
                    }
                }
                Some(_) => Err(CommandError::InvalidTarget {
                    reason: "rail target must be a Column node",
                }),
                None => Err(CommandError::MissingNode { node: column }),
            },
        }
    }
}

pub(super) fn allowed_area_rejection_reason(area: AllowedArea) -> &'static str {
    if area == AllowedArea::MAIN_DOCK {
        "panel constraints disallow the main dock area"
    } else if area == AllowedArea::FLOATING {
        "panel constraints disallow floating areas"
    } else if area == AllowedArea::ICON_RAIL {
        "panel constraints disallow icon rails"
    } else {
        "panel constraints disallow the requested dock area"
    }
}
