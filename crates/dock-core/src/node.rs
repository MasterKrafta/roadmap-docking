use crate::{Column, Panel, Split, Tabs};

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

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use crate::{Column, ColumnMode, Node, NodeId, Panel, PanelId, Tabs};

    #[test]
    fn a_tabs_node_references_panel_nodes_by_id() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let panel_id = PanelId::null();
        let panel_node = nodes.insert(Node::Panel(Panel::new(panel_id)));
        let tabs_node = nodes.insert(Node::Tabs(Tabs {
            children: vec![panel_node],
            active_index: 0,
            collapsed: false,
        }));

        assert!(matches!(nodes.get(panel_node), Some(Node::Panel(_))));
        assert!(matches!(nodes.get(tabs_node), Some(Node::Tabs(tabs)) if tabs.children == vec![panel_node]));
    }

    #[test]
    fn column_remains_a_special_node_with_one_group() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let group = nodes.insert(Node::Tabs(Tabs {
            children: vec![NodeId::null()],
            active_index: 0,
            collapsed: false,
        }));
        let column = Node::Column(Column {
            width: 280.0,
            mode: ColumnMode::Expanded,
            groups: vec![group],
            group_ratios: vec![1.0],
        });
        assert!(matches!(column, Node::Column(_)));
    }
}
