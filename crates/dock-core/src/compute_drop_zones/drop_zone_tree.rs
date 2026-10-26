use super::*;

pub(crate) struct DropZoneTreeIndex {
    parent_by_node: HashMap<NodeId, NodeId>,
    panel_by_id: HashMap<PanelId, NodeId>,
}

impl DropZoneTreeIndex {
    pub(super) fn new(layout: &Layout) -> Self {
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

pub(crate) fn floating_owner(
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

pub(crate) fn main_side_for_node(
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

pub(crate) fn collect_main_columns(layout: &Layout, root: NodeId, output: &mut Vec<NodeId>) {
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
