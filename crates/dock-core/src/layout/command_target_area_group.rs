use super::*;
use crate::AllowedArea;

impl Layout {
    pub(super) fn allowed_area_for_group_target(&self, group: NodeId) -> Result<AllowedArea, CommandError> {
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
}
