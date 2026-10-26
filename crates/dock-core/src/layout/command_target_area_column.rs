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
}
