use super::*;

pub(crate) fn main_columns(
    layout: &Layout,
    rects: &LayoutRects,
    side: DockSide,
) -> Vec<(NodeId, Rect)> {
    let Some(root) = layout.main_area(side).map(|area| area.root) else {
        return Vec::new();
    };
    // Only root Columns or direct Column children of a horizontal main Split
    // are valid MainColumn/ColumnSlot/RailSlot targets in the command model.
    let column_ids: Vec<NodeId> = match layout.node(root) {
        Some(Node::Column(_)) => vec![root],
        Some(Node::Split(split))
            if split.axis == crate::Axis::X
                && split
                    .children
                    .iter()
                    .all(|child| matches!(layout.node(*child), Some(Node::Column(_)))) =>
        {
            split.children.clone()
        }
        _ => return Vec::new(),
    };
    column_ids
        .into_iter()
        .filter_map(|column_id| {
            rects
                .columns
                .iter()
                .find(|column| column.column == column_id)
                .map(|column| (column_id, column.rect))
        })
        .collect()
}

pub(crate) fn group_main_column(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    group: NodeId,
    left_columns: &[(NodeId, Rect)],
    right_columns: &[(NodeId, Rect)],
) -> Option<(DockSide, usize, Rect)> {
    let column_id = owner_column(layout, tree_index, group)?;
    let (side, columns) = if left_columns.iter().any(|(id, _)| *id == column_id) {
        (DockSide::Left, left_columns)
    } else if right_columns.iter().any(|(id, _)| *id == column_id) {
        (DockSide::Right, right_columns)
    } else {
        return None;
    };
    columns
        .iter()
        .position(|(id, _)| *id == column_id)
        .map(|index| (side, index, columns[index].1))
}

pub(crate) fn edge_group_for_column(layout: &Layout, column: NodeId, last: bool) -> Option<NodeId> {
    let Some(Node::Column(value)) = layout.node(column) else {
        return None;
    };
    if last {
        value.groups.last().copied()
    } else {
        value.groups.first().copied()
    }
}

pub(crate) fn edge_group_for_dock_edge(
    layout: &Layout,
    columns: &[(NodeId, Rect)],
    is_left: bool,
) -> Option<NodeId> {
    let (column, _) = if is_left {
        columns.first().copied()?
    } else {
        columns.last().copied()?
    };
    edge_group_for_column(layout, column, is_left)
}

pub(crate) fn can_add_column(
    layout: &Layout,
    tree_index: &DropZoneTreeIndex,
    source: DropSource,
    source_root: NodeId,
    left_columns: &[(NodeId, Rect)],
    right_columns: &[(NodeId, Rect)],
    metrics: &DockMetrics,
) -> bool {
    if layout.profile() != Profile::Photoshop
        || left_columns.len() + right_columns.len() < metrics.max_columns
    {
        return true;
    }
    let Some(column_id) = owner_column(layout, tree_index, source_root) else {
        return false;
    };
    if main_side_for_node(layout, tree_index, column_id).is_none() {
        return false;
    }
    let Some(Node::Column(column)) = layout.node(column_id) else {
        return false;
    };
    match source {
        DropSource::Group { group } => column.groups.len() == 1 && column.groups[0] == group,
        DropSource::Panel { panel } => {
            let Some(panel_node) = panel_node(tree_index, panel) else {
                return false;
            };
            let Some(group) = direct_parent(tree_index, panel_node) else {
                return false;
            };
            column.groups.len() == 1
                && column.groups[0] == group
                && matches!(layout.node(group), Some(Node::Tabs(tabs)) if tabs.children.len() == 1)
        }
    }
}
