use crate::NodeId;

/// An ordered tab stack whose entries refer to nodes in the layout arena.
///
/// Structural and profile-specific constraints are checked by layout validation,
/// not by this passive data type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tabs {
    pub children: Vec<NodeId>,
    pub active_index: usize,
    pub collapsed: bool,
}

impl Tabs {
    /// Returns the active child when the stack has a child at active_index.
    ///
    /// Invalid indices and empty stacks return None; validation reports them
    /// as invalid persistent layout state.
    pub fn active_child(&self) -> Option<NodeId> {
        self.children.get(self.active_index).copied()
    }
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use crate::NodeId;
    use super::Tabs;

    #[test]
    fn active_child_uses_the_ordered_node_ids() {
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let first = nodes.insert(());
        let second = nodes.insert(());
        let tabs = Tabs {
            children: vec![first, second],
            active_index: 1,
            collapsed: false,
        };
        assert_eq!(tabs.active_child(), Some(second));
    }

    #[test]
    fn empty_or_out_of_range_active_index_has_no_active_child() {
        let empty = Tabs {
            children: Vec::new(),
            active_index: 0,
            collapsed: false,
        };
        assert_eq!(empty.active_child(), None);

        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let only = nodes.insert(());
        let out_of_range = Tabs {
            children: vec![only],
            active_index: 1,
            collapsed: false,
        };
        assert_eq!(out_of_range.active_child(), None);
    }
}
