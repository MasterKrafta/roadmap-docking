use std::collections::HashSet;

use crate::{ComputeRectsError, FloatingGroupRect, Layout, Node, NodeId, PanelId};

pub(super) fn floating_front_to_back(floating: &[FloatingGroupRect]) -> Vec<FloatingGroupRect> {
    let mut ordered = floating.to_vec();
    ordered.sort_by_key(|item| std::cmp::Reverse(item.z_order));
    ordered
}

fn collect_subtree(
    layout: &Layout,
    root: NodeId,
    nodes: &mut HashSet<NodeId>,
    panels: &mut HashSet<PanelId>,
) -> Result<(), ComputeRectsError> {
    if !nodes.insert(root) {
        return Ok(());
    }
    match layout.node(root).ok_or(ComputeRectsError::InvalidLayoutState)? {
        Node::Split(split) => {
            for child in &split.children {
                collect_subtree(layout, *child, nodes, panels)?;
            }
        }
        Node::Column(column) => {
            for group in &column.groups {
                collect_subtree(layout, *group, nodes, panels)?;
            }
        }
        Node::Tabs(tabs) => {
            for child in &tabs.children {
                collect_subtree(layout, *child, nodes, panels)?;
            }
        }
        Node::Panel(panel) => {
            panels.insert(panel.panel_id);
        }
    }
    Ok(())
}

pub(super) type FloatingSubtree = (HashSet<NodeId>, HashSet<PanelId>);

pub(super) fn floating_subtree(
    layout: &Layout,
    id: crate::FloatingGroupId,
) -> Result<Option<FloatingSubtree>, ComputeRectsError> {
    let Some(floating) = layout.floating_group(id) else {
        return Ok(None);
    };
    let Some(area) = layout.dock_area(floating.area) else {
        return Ok(None);
    };
    let mut nodes = HashSet::new();
    let mut panels = HashSet::new();
    collect_subtree(layout, area.root, &mut nodes, &mut panels)?;
    Ok(Some((nodes, panels)))
}

pub(super) fn all_floating_subtrees(
    layout: &Layout,
    floating: &[FloatingGroupRect],
) -> Result<(HashSet<NodeId>, HashSet<PanelId>), ComputeRectsError> {
    let mut nodes = HashSet::new();
    let mut panels = HashSet::new();
    for item in floating {
        let Some(group) = layout.floating_group(item.group) else {
            continue;
        };
        let Some(area) = layout.dock_area(group.area) else {
            continue;
        };

        // Collect directly into shared sets. Building a pair of fresh HashSets
        // per floating window and then extending the union repeats allocation
        // and hashing work on points outside every floating window.
        collect_subtree(layout, area.root, &mut nodes, &mut panels)?;
    }
    Ok((nodes, panels))
}
