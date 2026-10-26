use super::*;
use crate::{DragDropAction, DragDropRequest, NodeId};

impl Layout {
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
}
