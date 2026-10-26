use super::*;

pub(crate) fn target_area(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    target: NodeId,
) -> Option<AllowedArea> {
    if is_rail_group(layout, tree_index, target) {
        return Some(AllowedArea::ICON_RAIL);
    }
    if floating_owner(layout, tree_index, target).is_some() {
        return Some(AllowedArea::FLOATING);
    }
    main_side_for_node(layout, tree_index, target).map(|_| AllowedArea::MAIN_DOCK)
}

pub(crate) fn is_rail_group(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    group: NodeId,
) -> bool {
    matches!(
        owner_column(layout, tree_index, group).and_then(|column| layout.node(column)),
        Some(Node::Column(column)) if column.mode.is_rail()
    )
}

pub(crate) fn owner_column(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    node: NodeId,
) -> Option<NodeId> {
    let mut current = node;
    while let Some(parent) = direct_parent(tree_index, current) {
        if matches!(layout.node(parent), Some(Node::Column(_))) {
            return Some(parent);
        }
        current = parent;
    }
    None
}

fn floating_owner(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    node: NodeId,
) -> Option<FloatingGroupId> {
    layout.floating_groups().find_map(|(group_id, floating)| {
        layout
            .dock_area(floating.area)
            .filter(|area| contains_node(tree_index, area.root, node))
            .map(|_| group_id)
    })
}

pub(crate) fn source_is_floating_root(layout: &Layout, source_root: NodeId) -> bool {
    layout.floating_groups().any(|(_, floating)| {
        layout
            .dock_area(floating.area)
            .map(|area| area.root == source_root)
            .unwrap_or(false)
    })
}

pub(super) fn main_side_for_node(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    node: NodeId,
) -> Option<DockSide> {
    [DockSide::Left, DockSide::Right].into_iter().find(|side| {
        layout
            .main_area(*side)
            .map(|area| contains_node(tree_index, area.root, node))
            .unwrap_or(false)
    })
}
