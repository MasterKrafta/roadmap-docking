use super::*;

impl Layout {
    pub(super) fn panel_transfer_inverse(
        &self,
        panel: crate::PanelId,
        panel_node: NodeId,
    ) -> Option<DockCommand> {
        self.tab_parent_index(panel_node)
            .and_then(|(group, index)| {
                let source_group_has_other_tabs = matches!(
                    self.nodes.get(group),
                    Some(Node::Tabs(tabs)) if tabs.children.len() > 1
                );
                source_group_has_other_tabs.then_some(DockCommand::MovePanel {
                    panel,
                    target: DropTarget::Tabs { group, index },
                })
            })
    }
}
