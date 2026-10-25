use super::*;
use super::helpers::normalized_ratios;

impl Layout {
    pub(super) fn dock_floating_stack_to_main_column(
        &mut self,
        root: NodeId,
        split: Split,
        side: DockSide,
        index: usize,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if split
            .children
            .iter()
            .any(|id| !matches!(self.nodes.get(*id), Some(Node::Tabs(_))))
        {
            return Err(CommandError::InvalidTarget {
                reason: "floating stack contains a non-Tabs group",
            });
        }
        self.detach_node(root)?;
        self.nodes.remove(root);
        let column = self.nodes.insert(Node::Column(crate::Column {
            width: metrics.default_column_width,
            mode: ColumnMode::Expanded,
            groups: split.children,
            group_ratios: normalized_ratios(split.ratios.into_iter().map(Some).collect()),
            last_expanded_width: Some(metrics.default_column_width),
        }));
        self.insert_existing_column_at(side, index, column, None, metrics)
    }
}
