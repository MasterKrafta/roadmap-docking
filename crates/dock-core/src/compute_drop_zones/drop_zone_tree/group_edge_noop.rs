use super::*;

pub(crate) fn group_edge_is_noop(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    source: DropSource,
    target_group: NodeId,
    zone: DropZone,
) -> bool {
    let DropSource::Group {
        group: source_group,
    } = source
    else {
        return false;
    };
    if source_group == target_group || contains_node(tree_index, source_group, target_group) {
        return true;
    }
    let source_parent = direct_parent(tree_index, source_group);
    let target_parent = direct_parent(tree_index, target_group);
    if layout.adjacent_group_edge_is_noop(
        source_group,
        target_group,
        zone,
        source_parent,
        target_parent,
    ) {
        return true;
    }
    if matches!(zone, DropZone::Left | DropZone::Right) {
        let Some(source_column) = owner_column(layout, tree_index, source_group) else {
            return false;
        };
        let Some(target_column) = owner_column(layout, tree_index, target_group) else {
            return false;
        };
        if source_column == target_column {
            return false;
        }
        let (Some(source_side), Some(target_side)) = (
            main_side_for_node(layout, tree_index, source_column),
            main_side_for_node(layout, tree_index, target_column),
        ) else {
            return false;
        };
        if source_side != target_side {
            return false;
        }
        let root = match layout.main_area(source_side) {
            Some(area) => area.root,
            None => return false,
        };
        let mut columns = Vec::new();
        collect_main_columns(layout, root, &mut columns);
        let source_index = columns.iter().position(|id| *id == source_column);
        let target_index = columns.iter().position(|id| *id == target_column);
        if let (Some(source_index), Some(target_index)) = (source_index, target_index) {
            let source_is_single = matches!(
                layout.node(source_column),
                Some(Node::Column(column))
                    if column.groups.len() == 1 && column.groups[0] == source_group
            );
            return source_is_single
                && ((zone == DropZone::Left && source_index + 1 == target_index)
                    || (zone == DropZone::Right && target_index + 1 == source_index));
        }
    }
    false
}

fn collect_main_columns(layout: &Layout, root: NodeId, output: &mut Vec<NodeId>) {
    match layout.node(root) {
        Some(Node::Column(_)) => output.push(root),
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_main_columns(layout, *child, output);
            }
        }
        Some(Node::Tabs(_) | Node::Panel(_)) | None => {}
    }
}
