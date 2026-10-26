use super::{
    MAX_WORKSPACE_NODES, MAX_WORKSPACE_TREE_DEPTH, WORKSPACE_FORMAT, WORKSPACE_VERSION,
    WorkspaceFile,
};
use crate::{WorkspaceFileError, WorkspaceLayout, WorkspaceNode};

impl WorkspaceFile {
    fn validate_header(&self) -> Result<(), WorkspaceFileError> {
        if self.format != WORKSPACE_FORMAT {
            return Err(WorkspaceFileError::UnsupportedFormat {
                found: self.format.clone(),
            });
        }
        if self.version != WORKSPACE_VERSION {
            return Err(WorkspaceFileError::UnsupportedVersion {
                found: self.version,
                supported: WORKSPACE_VERSION,
            });
        }
        Ok(())
    }
}

pub(super) fn validate_workspace_tree_limits(
    layout: &WorkspaceLayout,
) -> Result<(), WorkspaceFileError> {
    let mut pending = Vec::<(&WorkspaceNode, usize)>::new();
    pending.extend(layout.left.iter().map(|node| (node, 1)));
    pending.extend(layout.right.iter().map(|node| (node, 1)));
    pending.extend(
        layout
            .floating_groups
            .iter()
            .map(|group| (&group.root, 1)),
    );

    let mut count = 0usize;
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_WORKSPACE_TREE_DEPTH {
            return Err(WorkspaceFileError::Json(format!(
                "workspace tree exceeds the maximum depth of {MAX_WORKSPACE_TREE_DEPTH}"
            )));
        }
        count += 1;
        if count > MAX_WORKSPACE_NODES {
            return Err(WorkspaceFileError::Json(format!(
                "workspace contains more than the maximum of {MAX_WORKSPACE_NODES} nodes"
            )));
        }
        match node {
            WorkspaceNode::Split { children, .. } | WorkspaceNode::Tabs { children, .. } => {
                pending.extend(children.iter().map(|child| (child, depth + 1)));
            }
            WorkspaceNode::Column { groups, .. } => {
                pending.extend(groups.iter().map(|group| (group, depth + 1)));
            }
            WorkspaceNode::Panel { .. } => {}
        }
    }
    Ok(())
}

pub(super) fn migrate_v1_to_v2(mut file: WorkspaceFile) -> WorkspaceFile {
    // V1 could omit last_expanded_width. For an Expanded column, current width
    // is an unambiguous fallback; a rail width cannot reveal the former width.
    for root in [&mut file.layout.left, &mut file.layout.right]
        .into_iter()
        .flatten()
    {
        migrate_v1_node(root);
    }
    for floating in &mut file.layout.floating_groups {
        migrate_v1_node(&mut floating.root);
    }
    file.version = WORKSPACE_VERSION;
    file
}

fn migrate_v1_node(node: &mut WorkspaceNode) {
    match node {
        WorkspaceNode::Split { children, .. } | WorkspaceNode::Tabs { children, .. } => {
            for child in children {
                migrate_v1_node(child);
            }
        }
        WorkspaceNode::Column {
            width,
            mode,
            groups,
            last_expanded_width,
            ..
        } => {
            if *mode == crate::ColumnMode::Expanded && last_expanded_width.is_none() {
                *last_expanded_width = Some(*width);
            }
            for group in groups {
                migrate_v1_node(group);
            }
        }
        WorkspaceNode::Panel { .. } => {}
    }
}
