use super::*;

impl Layout {
    pub(super) fn is_group_in_icon_rail(&self, group: NodeId) -> bool {
        let Some(column_id) = self.find_owner_column(group) else {
            return false;
        };
        self.main_column_position(column_id).is_some()
            && matches!(self.nodes.get(column_id), Some(Node::Column(column)) if column.mode.is_rail())
    }
}
