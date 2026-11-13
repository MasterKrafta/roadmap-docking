//! Drag-and-drop target validation and request resolution.

use super::*;
use crate::{
    AllowedArea, DragDropAction, DragDropRequest, DragHandle, DragPayload, FloatingGroupId, NodeId,
    PanelId,
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

        if let Some(root) = self.simple_composition_root_for_drag_request(request) {
            return self.resolve_simple_composition_drag(request, root, metrics);
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
            (DragHandle::DockAreaHeader, DragPayload::DockArea { side }) => {
                self.resolve_dock_area_header_drag(request, side, metrics)
            }
            _ => Err(CommandError::InvalidTarget {
                reason: "drag handle and payload do not describe a supported drag operation",
            }),
        }
    }

    fn simple_composition_root_for_drag_request(
        &self,
        request: &DragDropRequest,
    ) -> Option<NodeId> {
        match (request.handle, request.payload) {
            (DragHandle::PanelTab, DragPayload::Panel { panel }) => {
                self.simple_composition_root_for_source(DropSource::Panel { panel })
            }
            (DragHandle::GroupGrip, DragPayload::Group { group }) => {
                self.simple_composition_root_for_source(DropSource::Group { group })
            }
            (DragHandle::DockAreaHeader, DragPayload::DockArea { side }) => {
                let root = self.main_area(side)?.root;
                self.simple_composition_root_for_source(DropSource::Composition { root })
            }
            (DragHandle::FloatingCapture, DragPayload::FloatingGroup { group }) => {
                let root = self
                    .floating_group(group)
                    .and_then(|floating| self.dock_area(floating.area))
                    .map(|area| area.root)?;
                self.simple_composition_root_for_source(DropSource::Composition { root })
            }
            _ => None,
        }
    }

    /// The exact 1/1/1 shape is one semantic drag source regardless of which
    /// surface initiated it. Keep the Column as the payload root whenever the
    /// destination accepts a complete Column; only vertical Group insertion
    /// consumes the structural wrapper and moves its sole Tabs child.
    fn resolve_simple_composition_drag(
        &self,
        request: &DragDropRequest,
        column: NodeId,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
        let group = self
            .simple_composition_group(column)
            .ok_or(CommandError::InvalidTarget {
                reason: "simple composition source no longer has the 1/1/1 shape",
            })?;
        let Some(target) = request.target else {
            return if matches!(request.handle, DragHandle::FloatingCapture) {
                Ok(DragDropAction::NoOp)
            } else {
                Err(CommandError::InvalidTarget {
                    reason: "simple-composition drag requires a resolved drop target",
                })
            };
        };

        if target
            .referenced_node_id()
            .is_some_and(|node| self.is_descendant_or_same(column, node))
        {
            return Ok(DragDropAction::NoOp);
        }

        if matches!(target, DropTarget::FreeArea) {
            if self.find_floating_root_containing(column).is_some() {
                return Err(CommandError::InvalidTarget {
                    reason: "a Floating composition cannot be floated into its own free area",
                });
            }
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
                &self.panel_ids_in_subtree(column),
                AllowedArea::FLOATING,
            )?;
            return Ok(DragDropAction::Command(DockCommand::Float {
                node: column,
                bounds,
            }));
        }

        self.validate_drop_target(target, metrics)?;
        self.validate_panel_ids_allowed_for_target(&self.panel_ids_in_subtree(column), target)?;

        match target {
            DropTarget::MainColumn { side, index } => {
                if let Some((source_side, source_index)) = self.main_column_position(column)
                    && source_side == side
                    && (index == source_index || index == source_index + 1)
                {
                    return Ok(DragDropAction::NoOp);
                }
                Ok(DragDropAction::Command(
                    DockCommand::MoveColumnToMainColumn {
                        column,
                        side,
                        index,
                    },
                ))
            }
            DropTarget::GroupEdge {
                group: target_group,
                zone: zone @ (
                    DropZone::Left | DropZone::Right | DropZone::FloatingColumnInsertion
                ),
            } => {
                if self.group_edge_is_same_position(group, target_group, zone) {
                    return Ok(DragDropAction::NoOp);
                }
                Ok(DragDropAction::Command(DockCommand::MoveColumnToTarget {
                    column,
                    target,
                }))
            }
            DropTarget::GroupEdge {
                group: target_group,
                zone: zone @ (DropZone::Top | DropZone::Bottom),
            } => {
                if self.group_edge_is_same_position(group, target_group, zone) {
                    return Ok(DragDropAction::NoOp);
                }
                Ok(DragDropAction::Command(DockCommand::SplitGroup {
                    group,
                    target_group,
                    zone,
                    ratio: 0.5,
                }))
            }
            DropTarget::GroupEdge { .. } => Err(CommandError::InvalidTarget {
                reason: "simple-composition group-edge target requires a cardinal edge zone",
            }),
            DropTarget::GroupTabs {
                group: target_group,
                index,
            }
            | DropTarget::Tabs {
                group: target_group,
                index,
            } => {
                if group == target_group {
                    return Ok(DragDropAction::NoOp);
                }
                Ok(DragDropAction::Command(DockCommand::MergeGroup {
                    group,
                    target_group,
                    index,
                }))
            }
            DropTarget::ColumnSlot {
                column: target_column,
                index,
            } => {
                if self.group_slot_is_noop(group, target_column, index) {
                    return Ok(DragDropAction::NoOp);
                }
                Ok(DragDropAction::Command(DockCommand::MoveGroup {
                    group,
                    column: target_column,
                    index,
                }))
            }
            DropTarget::RailSlot {
                column: target_column,
                index,
            } => {
                let group_index = self.rail_slot_group_index(target_column, index)?;
                if self.group_slot_is_noop(group, target_column, group_index) {
                    return Ok(DragDropAction::NoOp);
                }
                Ok(DragDropAction::Command(DockCommand::MoveGroup {
                    group,
                    column: target_column,
                    index: group_index,
                }))
            }
            DropTarget::FreeArea => unreachable!("FreeArea is handled before target validation"),
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
        let Some(target) = request.target else {
            return if self.floating_group(group).is_some() {
                Ok(DragDropAction::NoOp)
            } else {
                Err(CommandError::MissingFloatingGroup { group })
            };
        };
        if matches!(target, DropTarget::FreeArea) {
            return Err(CommandError::InvalidTarget {
                reason: "a floating capture drag requires a dock/stack target",
            });
        }
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
        self.validate_floating_dock_target(root, target, metrics)?;
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

        if let Some(action) = self.resolve_panel_drag_noop(request, panel, panel_node, target) {
            return Ok(action);
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

    /// Returns an action for panel drops that leave the dock in the same position.
    ///
    /// Flyout drags still produce their close-on-release command, while ordinary drags
    /// resolve to an explicit no-op.
    fn resolve_panel_drag_noop(
        &self,
        request: &DragDropRequest,
        panel: PanelId,
        panel_node: NodeId,
        target: DropTarget,
    ) -> Option<DragDropAction> {
        if let (Some((source_group, source_index)), DropTarget::Tabs { group, index }) =
            (self.tab_parent_index(panel_node), target)
            && source_group == group
            && (index == source_index || index == source_index + 1)
        {
            return Some(if request.handle == DragHandle::FlyoutPanel {
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
                if let DropTarget::GroupEdge {
                    group: target_group,
                    zone: DropZone::FloatingColumnInsertion,
                } = target
                    && source_group_has_only_panel
                    && source_group == target_group
                {
                    return Some(if request.handle == DragHandle::FlyoutPanel {
                        DragDropAction::Command(DockCommand::MovePanelFromFlyout {
                            panel,
                            target,
                        })
                    } else {
                        DragDropAction::NoOp
                    });
                }
                if source_column_has_only_group
                    && let Some((source_side, source_index)) =
                        self.main_column_position(source_column)
                {
                    match target {
                        DropTarget::MainColumn { side, index }
                            if side == source_side
                                && (index == source_index || index == source_index + 1) =>
                        {
                            return Some(if request.handle == DragHandle::FlyoutPanel {
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
                            return Some(if request.handle == DragHandle::FlyoutPanel {
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
        // Dropping a panel on its own group's edge leaves the layout unchanged.
        // A flyout release still uses the flyout command so the transient flyout closes.
        if let (
            Some((source_group, _)),
            DropTarget::GroupEdge {
                group: target_group,
                zone,
            },
        ) = (self.tab_parent_index(panel_node), target)
            && source_group == target_group
            && zone != DropZone::FloatingColumnInsertion
        {
            return Some(if request.handle == DragHandle::FlyoutPanel {
                DragDropAction::Command(DockCommand::MovePanelFromFlyout { panel, target })
            } else {
                DragDropAction::NoOp
            });
        }

        None
    }

    /// Resolves a whole Main DockArea drag. A single-Group composition reuses
    /// Group docking semantics; a multi-Group or multi-Column composition can
    /// dock into an existing Floating composition without being split apart
    /// before the destination-specific operation is selected.
    fn resolve_dock_area_header_drag(
        &self,
        request: &DragDropRequest,
        side: DockSide,
        metrics: &DockMetrics,
    ) -> Result<DragDropAction, CommandError> {
        let root = self.main_area(side).map(|area| area.root).ok_or(
            CommandError::InvalidTarget {
                reason: "DockArea drag source no longer exists",
            },
        )?;
        let target = request.target.ok_or(CommandError::InvalidTarget {
            reason: "DockArea header drag requires a resolved drop target",
        })?;

        if target == crate::DropTarget::FreeArea {
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
                &self.panel_ids_in_subtree(root),
                AllowedArea::FLOATING,
            )?;
            return Ok(DragDropAction::Command(DockCommand::Float {
                node: root,
                bounds,
            }));
        }

        if let Some(group) = single_group_in_subtree(self, root) {
            return self.resolve_group_grip_drag(request, group, metrics);
        }

        let supported_floating_target = match target {
            DropTarget::GroupEdge {
                zone: DropZone::Left
                    | DropZone::Right
                    | DropZone::Top
                    | DropZone::Bottom
                    | DropZone::FloatingColumnInsertion,
                ..
            } => true,
            DropTarget::ColumnSlot { .. } => matches!(self.node(root), Some(Node::Column(_))),
            _ => false,
        };
        if self.profile == crate::Profile::Photoshop
            && self.is_column_composition_root(root)
            && supported_floating_target
            && target
                .referenced_node_id()
                .is_some_and(|node| self.find_floating_root_containing(node).is_some())
        {
            self.validate_floating_dock_target(root, target, metrics)?;
            self.validate_panel_ids_allowed_for_target(&self.panel_ids_in_subtree(root), target)?;
            return Ok(DragDropAction::Command(DockCommand::DockMainArea {
                side,
                target,
            }));
        }

        Err(CommandError::InvalidTarget {
            reason: "a DockArea with multiple groups can only be detached as one floating composition",
        })
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
        self.validate_panel_ids_allowed_for_target(&self.panel_ids_in_subtree(group), target)?;

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
        if zone == DropZone::FloatingColumnInsertion {
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
                Some(Node::Column(column))
                    if column.groups.len() == 1 && column.groups[0] == source_group
            );
            if !source_is_sole_group {
                return false;
            }
            let Some(target_root) = self.find_floating_root_containing(target_column) else {
                return false;
            };
            if self.find_floating_root_containing(source_column) != Some(target_root) {
                return false;
            }
            let Some(Node::Split(split)) = self.node(target_root) else {
                return false;
            };
            if split.axis != Axis::X
                || !split
                    .children
                    .iter()
                    .all(|child| matches!(self.node(*child), Some(Node::Column(_))))
            {
                return false;
            }
            let (Some(source_index), Some(target_index)) = (
                split.children.iter().position(|child| *child == source_column),
                split.children.iter().position(|child| *child == target_column),
            ) else {
                return false;
            };
            return source_index.checked_add(1) == Some(target_index);
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
                Some(Node::Column(column)) if column.mode.is_rail() => Ok(AllowedArea::ICON_RAIL),
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

    pub(super) fn allowed_area_for_column_slot_target(
        &self,
        column: NodeId,
    ) -> Result<AllowedArea, CommandError> {
        let Some(Node::Column(value)) = self.nodes.get(column) else {
            return Err(if self.nodes.contains_key(column) {
                CommandError::InvalidTarget {
                    reason: "column slot target must be a Column node",
                }
            } else {
                CommandError::MissingNode { node: column }
            });
        };

        if value.mode.is_rail() {
            return Err(CommandError::InvalidTarget {
                reason: "expanded-column insertion target cannot be a rail; use RailSlot",
            });
        }

        if self.main_column_position(column).is_some() {
            return Ok(AllowedArea::MAIN_DOCK);
        }

        if self.find_floating_root_containing(column).is_some() {
            return Ok(AllowedArea::FLOATING);
        }

        Err(CommandError::InvalidTarget {
            reason: "column insertion target must belong to a main or floating dock area",
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
            DropTarget::MainColumn { .. } => Ok(AllowedArea::MAIN_DOCK),
            DropTarget::ColumnSlot { column, .. } => self.allowed_area_for_column_slot_target(column),
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

    fn floating_column_composition_edge_is_valid(
        &self,
        column: NodeId,
        zone: DropZone,
    ) -> bool {
        let Some(root) = self.find_floating_root_containing(column) else {
            return false;
        };
        match self.nodes.get(root) {
            Some(Node::Column(_)) => root == column,
            Some(Node::Split(split))
                if split.axis == Axis::X
                    && split
                        .children
                        .iter()
                        .all(|child| matches!(self.nodes.get(*child), Some(Node::Column(_)))) =>
            {
                let Some(index) = split.children.iter().position(|child| *child == column) else {
                    return false;
                };
                if zone == DropZone::Left {
                    index == 0
                } else if zone == DropZone::Right {
                    index + 1 == split.children.len()
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn floating_column_insertion_boundary_is_valid(
        &self,
        group: NodeId,
    ) -> bool {
        let Some(column) = self.find_owner_column(group) else {
            return false;
        };
        let Some(Node::Column(column_data)) = self.node(column) else {
            return false;
        };
        if column_data.mode.is_rail() || column_data.groups.first() != Some(&group) {
            return false;
        }
        let Some(root) = self.find_floating_root_containing(column) else {
            return false;
        };
        let Some(Node::Split(split)) = self.node(root) else {
            return false;
        };
        if split.axis != Axis::X
            || !split
                .children
                .iter()
                .all(|child| matches!(self.node(*child), Some(Node::Column(_))))
        {
            return false;
        }
        let Some(index) = split.children.iter().position(|child| *child == column) else {
            return false;
        };
        if index == 0 {
            return false;
        }
        !matches!(
            split.children.get(index - 1).and_then(|id| self.node(*id)),
            Some(Node::Column(previous)) if previous.mode.is_rail()
        )
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
                    DropZone::Left
                        | DropZone::Right
                        | DropZone::Top
                        | DropZone::Bottom
                        | DropZone::FloatingColumnInsertion
                ) {
                    return Err(CommandError::InvalidTarget {
                        reason: "group-edge target requires a cardinal edge or Floating column insertion zone",
                    });
                }
                if zone == DropZone::FloatingColumnInsertion && self.profile != Profile::Photoshop {
                    return Err(CommandError::InvalidTarget {
                        reason: "Floating column insertion is available only in the Photoshop profile",
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
                        DropZone::FloatingColumnInsertion => {
                            if !self.floating_column_insertion_boundary_is_valid(group) {
                                return Err(CommandError::InvalidTarget {
                                    reason: "Floating column insertion requires an internal boundary of a Split(X) composition",
                                });
                            }
                        }
                        DropZone::Left | DropZone::Right => {
                            let Some(column) = self.find_owner_column(group) else {
                                return Err(CommandError::InvalidTarget {
                                    reason: if self.find_floating_root_containing(group).is_some() {
                                        "horizontal splits are not supported inside a floating stack"
                                    } else {
                                        "horizontal group splits require a dock Column"
                                    },
                                });
                            };
                            if self.main_column_position(column).is_none()
                                && !self.floating_column_composition_edge_is_valid(column, zone)
                            {
                                return Err(CommandError::InvalidTarget {
                                    reason: "horizontal group splits require a Main Column or the outer edge of a Floating Column composition",
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
            DropTarget::ColumnSlot { column, index } => {
                self.allowed_area_for_column_slot_target(column)?;
                match self.nodes.get(column) {
                    Some(Node::Column(value)) if index <= value.groups.len() => Ok(()),
                    Some(Node::Column(_)) => Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "column group insertion index is outside the column",
                    }),
                    Some(_) => Err(CommandError::InvalidTarget {
                        reason: "column slot target must be a Column node",
                    }),
                    None => Err(CommandError::MissingNode { node: column }),
                }
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


fn single_group_in_subtree(layout: &Layout, root: NodeId) -> Option<NodeId> {
    let mut stack = vec![root];
    let mut group = None;

    while let Some(node_id) = stack.pop() {
        match layout.node(node_id)? {
            Node::Tabs(_) => {
                if group.replace(node_id).is_some() {
                    return None;
                }
            }
            node => stack.extend(node.children().iter().copied()),
        }
    }

    group
}
