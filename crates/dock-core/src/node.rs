use crate::{Column, NodeId, Panel, Split, Tabs};

/// One node in the layout tree.
///
/// Panel identity is stored once in a leaf node; Tabs and Column reference
/// children by NodeId rather than embedding duplicate node values.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Split(Split),
    Tabs(Tabs),
    Panel(Panel),
    Column(Column),
}

impl Node {
    /// Returns direct child IDs in their stored order. Panel nodes are leaves.
    ///
    /// This accessor only describes tree structure; callers remain responsible
    /// for validating child references and the surrounding layout.
    pub(crate) fn children(&self) -> &[NodeId] {
        match self {
            Self::Split(split) => &split.children,
            Self::Tabs(tabs) => &tabs.children,
            Self::Column(column) => &column.groups,
            Self::Panel(_) => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use crate::{Column, ColumnMode, Node, NodeId, Panel, PanelId, Split, Tabs};

    #[test]
    fn children_returns_direct_ids_in_stored_order_for_each_node_kind() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let mut panels = SlotMap::<PanelId, ()>::with_key();
        let first = nodes.insert(Node::Panel(Panel::new(panels.insert(()))));
        let second = nodes.insert(Node::Panel(Panel::new(panels.insert(()))));

        let split = Node::Split(Split {
            axis: crate::Axis::X,
            children: vec![first, second],
            ratios: vec![0.25, 0.75],
        });
        let tabs = Node::Tabs(Tabs {
            children: vec![second, first],
            active_index: 0,
            collapsed: false,
        });
        let column = Node::Column(Column {
            width: 280.0,
            mode: ColumnMode::Expanded,
            groups: vec![second, first],
            group_ratios: vec![0.75, 0.25],
            last_expanded_width: Some(280.0),
        });

        assert!(nodes[first].children().is_empty());
        assert_eq!(split.children(), &[first, second]);
        assert_eq!(tabs.children(), &[second, first]);
        assert_eq!(column.children(), &[second, first]);
    }

    #[test]
    fn a_tabs_node_references_panel_nodes_by_id() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let mut panels = SlotMap::<PanelId, ()>::with_key();
        let panel_id = panels.insert(());
        let panel_node = nodes.insert(Node::Panel(Panel::new(panel_id)));
        let tabs_node = nodes.insert(Node::Tabs(Tabs {
            children: vec![panel_node],
            active_index: 0,
            collapsed: false,
        }));

        assert!(matches!(nodes.get(panel_node), Some(Node::Panel(_))));
        assert!(
            matches!(nodes.get(tabs_node), Some(Node::Tabs(tabs)) if tabs.children == vec![panel_node])
        );
    }

    #[test]
    fn column_remains_a_special_node_with_one_group() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let mut panels = SlotMap::<PanelId, ()>::with_key();
        let panel_id = panels.insert(());
        let panel_node = nodes.insert(Node::Panel(Panel::new(panel_id)));
        let group = nodes.insert(Node::Tabs(Tabs {
            children: vec![panel_node],
            active_index: 0,
            collapsed: false,
        }));
        let column = Node::Column(Column {
            width: 280.0,
            mode: ColumnMode::Expanded,
            groups: vec![group],
            group_ratios: vec![1.0],
            last_expanded_width: Some(280.0),
        });
        assert!(matches!(column, Node::Column(_)));
    }
}
