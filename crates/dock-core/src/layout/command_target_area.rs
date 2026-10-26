use super::*;
use crate::AllowedArea;

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
