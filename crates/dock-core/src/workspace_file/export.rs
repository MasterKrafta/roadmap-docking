use super::{MAX_WORKSPACE_TREE_DEPTH, WorkspaceFile};
use crate::{
    DockSide, Layout, Node, NodeId, WorkspaceFileError, WorkspaceFloatingGroup, WorkspaceLayout,
    WorkspaceNode,
};

impl WorkspaceFile {
    pub(super) fn export_layout_unchecked(layout: &Layout) -> Result<Self, WorkspaceFileError> {
        let left = layout
            .main_area(DockSide::Left)
            .map(|area| node_to_workspace(layout, area.root))
            .transpose()?;
        let right = layout
            .main_area(DockSide::Right)
            .map(|area| node_to_workspace(layout, area.root))
            .transpose()?;

        let mut floating_groups = Vec::new();
        for (group_id, group) in layout.floating_groups_in_paint_order() {
            let area = layout.dock_area(group.area).ok_or_else(|| {
                WorkspaceFileError::InvalidRuntimeLayout(format!(
                    "floating group {group_id:?} references a missing dock area"
                ))
            })?;
            area.validate_metadata()
                .map_err(|reason| WorkspaceFileError::InvalidRuntimeLayout(reason.to_owned()))?;
            let root = node_to_workspace(layout, area.root)?;
            floating_groups.push(WorkspaceFloatingGroup {
                root,
                bounds: group.bounds,
                z_order: group.z_order,
            });
        }

        Ok(Self::new(WorkspaceLayout {
            profile: layout.profile(),
            chrome: layout.chrome(),
            canvas: *layout.canvas(),
            left,
            right,
            floating_groups,
        }))
    }
}

fn node_to_workspace(
    layout: &Layout,
    node_id: NodeId,
) -> Result<WorkspaceNode, WorkspaceFileError> {
    node_to_workspace_at_depth(layout, node_id, 1)
}

fn node_to_workspace_at_depth(
    layout: &Layout,
    node_id: NodeId,
    depth: usize,
) -> Result<WorkspaceNode, WorkspaceFileError> {
    if depth > MAX_WORKSPACE_TREE_DEPTH {
        return Err(WorkspaceFileError::InvalidRuntimeLayout(format!(
            "layout tree exceeds the maximum workspace depth of {MAX_WORKSPACE_TREE_DEPTH}"
        )));
    }
    let node = layout.node(node_id).ok_or_else(|| {
        WorkspaceFileError::InvalidRuntimeLayout(
            "a dock area or child references a missing node".to_owned(),
        )
    })?;

    match node {
        Node::Split(split) => Ok(WorkspaceNode::Split {
            axis: split.axis,
            children: split
                .children
                .iter()
                .map(|child| node_to_workspace_at_depth(layout, *child, depth + 1))
                .collect::<Result<_, _>>()?,
            ratios: split.ratios.clone(),
        }),
        Node::Tabs(tabs) => Ok(WorkspaceNode::Tabs {
            children: tabs
                .children
                .iter()
                .map(|child| node_to_workspace_at_depth(layout, *child, depth + 1))
                .collect::<Result<_, _>>()?,
            active_index: tabs.active_index,
            collapsed: tabs.collapsed,
        }),
        Node::Panel(panel) => {
            let instance = layout.panel_instance(panel.panel_id).ok_or_else(|| {
                WorkspaceFileError::InvalidRuntimeLayout(
                    "a panel leaf references a missing panel instance".to_owned(),
                )
            })?;
            Ok(WorkspaceNode::Panel {
                key: instance.key.clone(),
                state: instance.state,
            })
        }
        Node::Column(column) => Ok(WorkspaceNode::Column {
            width: column.width,
            mode: column.mode,
            groups: column
                .groups
                .iter()
                .map(|group| node_to_workspace_at_depth(layout, *group, depth + 1))
                .collect::<Result<_, _>>()?,
            group_ratios: column.group_ratios.clone(),
            last_expanded_width: column.last_expanded_width,
        }),
    }
}
