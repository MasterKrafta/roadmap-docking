use super::*;

impl Layout {
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
