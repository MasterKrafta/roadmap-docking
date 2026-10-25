use super::*;

impl Layout {
    pub(super) fn dock_floating_root_to_target(
        &mut self,
        root: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        self.detach_node(root)?;
        self.insert_node_for_target(root, target, metrics, Some(0.5))
    }
}
