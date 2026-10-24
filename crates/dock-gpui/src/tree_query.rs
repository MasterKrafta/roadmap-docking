use std::collections::HashSet;

use dock_core::{Axis, DockSide, DropTarget, Layout, Node, NodeId, PanelId, Tabs};

/// Finds one top Tabs group per vertical branch along the floating root's
/// upper edge. A Column contributes its first group; a horizontal Split
/// contributes the first group from each child branch.
pub(crate) fn floating_title_groups_in_tree(layout: &Layout, root: NodeId) -> Vec<NodeId> {
    match layout.node(root) {
        Some(Node::Tabs(_)) => vec![root],
        Some(Node::Column(column)) => {
            column.groups.first().copied().into_iter().collect()
        }
        Some(Node::Split(split)) if split.axis == Axis::X => split
            .children
            .iter()
            .flat_map(|child| floating_title_groups_in_tree(layout, *child))
            .collect(),
        Some(Node::Split(split)) => split
            .children
            .first()
            .map(|child| floating_title_groups_in_tree(layout, *child))
            .unwrap_or_default(),
        Some(Node::Panel(_)) | None => Vec::new(),
    }
}

pub(crate) fn drop_target_node(target: DropTarget) -> Option<NodeId> {
    match target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => Some(group),
        DropTarget::ColumnSlot { column, .. } | DropTarget::RailSlot { column, .. } => {
            Some(column)
        }
        DropTarget::MainColumn { .. } | DropTarget::FreeArea => None,
    }
}

pub(crate) fn main_column_ids(layout: &Layout, side: DockSide) -> Vec<NodeId> {
    let Some(area) = layout.main_area(side) else {
        return Vec::new();
    };
    match layout.node(area.root) {
        Some(Node::Column(_)) => vec![area.root],
        Some(Node::Split(split))
            if split.axis == Axis::X
                && split.children.iter().all(|child| matches!(layout.node(*child), Some(Node::Column(_)))) =>
        {
            split.children.clone()
        }
        _ => Vec::new(),
    }
}

pub(crate) fn first_panel_in_subtree(layout: &Layout, node_id: NodeId) -> Option<PanelId> {
    match layout.node(node_id)? {
        Node::Panel(panel) => Some(panel.panel_id),
        Node::Tabs(tabs) => active_panel(layout, tabs).or_else(|| {
            tabs.children
                .first()
                .and_then(|child| first_panel_in_subtree(layout, *child))
        }),
        Node::Column(column) => column
            .groups
            .first()
            .and_then(|group| first_panel_in_subtree(layout, *group)),
        Node::Split(split) => split
            .children
            .first()
            .and_then(|child| first_panel_in_subtree(layout, *child)),
    }
}

pub(crate) fn subtree_nodes(layout: &Layout, root: NodeId) -> HashSet<NodeId> {
    let mut result = HashSet::new();
    let mut pending = vec![root];

    while let Some(id) = pending.pop() {
        if !result.insert(id) {
            continue;
        }

        let Some(node) = layout.node(id) else {
            continue;
        };
        match node {
            Node::Split(split) => pending.extend(split.children.iter().copied()),
            Node::Tabs(tabs) => pending.extend(tabs.children.iter().copied()),
            Node::Column(column) => pending.extend(column.groups.iter().copied()),
            Node::Panel(_) => {}
        }
    }

    result
}

pub(crate) fn active_panel(layout: &Layout, tabs: &Tabs) -> Option<PanelId> {
    let node_id = tabs.active_child()?;
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => Some(panel.panel_id),
        _ => None,
    }
}

