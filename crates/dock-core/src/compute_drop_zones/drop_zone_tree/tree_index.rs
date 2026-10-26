use super::*;

pub(crate) struct DropZoneTreeIndex {
    parent_by_node: HashMap<NodeId, NodeId>,
    panel_by_id: HashMap<PanelId, NodeId>,
}

impl DropZoneTreeIndex {
    pub(in crate::compute_drop_zones) fn new(layout: &Layout) -> Self {
        let mut parent_by_node = HashMap::new();
        let mut panel_by_id = HashMap::new();

        for (node_id, node) in layout.nodes() {
            if let Node::Panel(panel) = node {
                panel_by_id.insert(panel.panel_id, node_id);
            }
            for child in node.children() {
                parent_by_node.insert(*child, node_id);
            }
        }

        Self { parent_by_node, panel_by_id }
    }

    pub(super) fn contains_node(&self, root: NodeId, target: NodeId) -> bool {
        let mut current = target;
        loop {
            if current == root {
                return true;
            }
            let Some(parent) = self.parent_by_node.get(&current) else {
                return false;
            };
            current = *parent;
        }
    }

    pub(super) fn direct_parent(&self, child: NodeId) -> Option<NodeId> {
        self.parent_by_node.get(&child).copied()
    }
}

pub(crate) fn source_root(
    layout: &Layout,
    source: DropSource,
    tree_index: &DropZoneTreeIndex,
) -> Result<NodeId, ComputeRectsError> {
    match source {
        DropSource::Panel { panel } => {
            panel_node(tree_index, panel).ok_or(ComputeRectsError::InvalidLayoutState)
        }
        DropSource::Group { group } => {
            if matches!(layout.node(group), Some(Node::Tabs(_))) {
                Ok(group)
            } else {
                Err(ComputeRectsError::InvalidLayoutState)
            }
        }
    }
}

pub(crate) fn panel_node(tree_index: &DropZoneTreeIndex, panel: PanelId) -> Option<NodeId> {
    tree_index.panel_by_id.get(&panel).copied()
}

pub(crate) fn collect_panels(layout: &Layout, root: NodeId) -> Vec<PanelId> {
    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(node_id) = stack.pop() {
        let Some(node) = layout.node(node_id) else {
            continue;
        };
        if let Node::Panel(panel) = node {
            result.push(panel.panel_id);
        }
        stack.extend(node.children().iter().copied());
    }
    result
}

pub(crate) fn source_allows(layout: &Layout, panels: &[PanelId], area: AllowedArea) -> bool {
    panels.iter().all(|panel| {
        layout
            .panel_instance(*panel)
            .map(|instance| instance.constraints.allows_area(area))
            .unwrap_or(false)
    })
}

pub(crate) fn contains_node(tree_index: &DropZoneTreeIndex, root: NodeId, target: NodeId) -> bool {
    tree_index.contains_node(root, target)
}

pub(crate) fn direct_parent(tree_index: &DropZoneTreeIndex, child: NodeId) -> Option<NodeId> {
    tree_index.direct_parent(child)
}
