use super::*;

#[cfg(test)]
mod input_validation_tests {
    use super::*;
    use crate::{Column, ColumnMode, Tabs};

    #[test]
    fn group_slot_noop_requires_the_actual_owner_and_a_valid_adjacent_slot() {
        let mut layout = Layout::new(Profile::Generic);
        let group = layout.nodes.insert(Node::Tabs(Tabs {
            children: Vec::new(),
            active_index: 0,
            collapsed: false,
        }));
        let column_node = || {
            Node::Column(Column {
                width: 280.0,
                mode: ColumnMode::Expanded,
                groups: vec![group],
                group_ratios: vec![1.0],
                last_expanded_width: Some(280.0),
            })
        };
        let owner = layout.nodes.insert(column_node());
        let duplicate_owner = layout.nodes.insert(column_node());

        // Invalid duplicate ownership must not make the second column report
        // a no-op. Valid insertion slots remain no-ops, while an invalid index
        // is left for the command validator to reject.
        assert_eq!(layout.find_owner_column(group), Some(owner));
        assert!(!layout.group_slot_is_noop(group, duplicate_owner, 0));
        assert!(layout.group_slot_is_noop(group, owner, 0));
        assert!(layout.group_slot_is_noop(group, owner, 1));
        assert!(!layout.group_slot_is_noop(group, owner, 2));
    }
}
