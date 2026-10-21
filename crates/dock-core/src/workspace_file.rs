use std::collections::HashMap;

use slotmap::SlotMap;

use crate::{
    CanvasState, ChromeMode, Column, DockArea, DockAreaId, DockMetrics, DockSide, FloatingGroup,
    FloatingGroupId, Layout, Node, NodeId, Panel, PanelConstraints, PanelId, PanelInstance, PanelKey,
    PanelState, Profile, Split, Tabs, ValidationError, WorkspaceFileError, WorkspaceFloatingGroup,
    WorkspaceLayout, WorkspaceNode,
};

/// Marker identifying this file as a Roadmap Docking workspace.
pub const WORKSPACE_FORMAT: &str = "roadmap-docking-workspace";

/// Current public workspace JSON schema version.
pub const WORKSPACE_VERSION: u32 = 1;

/// Versioned, JSON-encoded persistent workspace boundary.
///
/// Persist and restore this type, not Layout or individual arena-backed model
/// fragments. The nested DTO tree has no SlotMap handles, and loading always
/// creates fresh runtime arenas. JSON is the sole public workspace file format.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFile {
    pub format: String,
    pub version: u32,
    pub layout: WorkspaceLayout,
}

impl WorkspaceFile {
    /// Wraps persistent layout data in the current versioned file envelope.
    pub fn new(layout: WorkspaceLayout) -> Self {
        Self {
            format: WORKSPACE_FORMAT.to_owned(),
            version: WORKSPACE_VERSION,
            layout,
        }
    }

    /// Captures persistent state without exposing runtime arena keys or UI state.
    pub fn from_layout(layout: &Layout) -> Result<Self, WorkspaceFileError> {
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
            area.validate_metadata().map_err(|reason| {
                WorkspaceFileError::InvalidRuntimeLayout(reason.to_owned())
            })?;
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

    /// Parses a workspace file and rejects unsupported format/version headers.
    pub fn from_json(json: &str) -> Result<Self, WorkspaceFileError> {
        let file: Self = serde_json::from_str(json)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))?;
        file.validate_header()?;
        Ok(file)
    }

    /// Encodes this workspace as the canonical pretty-printed JSON file.
    pub fn to_json_pretty(&self) -> Result<String, WorkspaceFileError> {
        self.validate_header()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))
    }

    /// Rebuilds fresh runtime arenas using the application's panel descriptors.
    ///
    /// Panel constraints come from the app-owned registry and are deliberately
    /// not persisted. This is the strict version-1 loader; migration and
    /// recovery policy are layered on top in the workspace-load stage.
    pub fn into_layout(
        &self,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
    ) -> Result<Layout, WorkspaceFileError> {
        self.validate_header()?;
        metrics
            .validate()
            .map_err(WorkspaceFileError::InvalidMetrics)?;

        let mut descriptors_by_key = HashMap::<String, PanelConstraints>::new();
        for (key, constraints) in descriptors {
            constraints.validate().map_err(|reason| {
                WorkspaceFileError::InvalidPanelDescriptor {
                    key: key.as_str().to_owned(),
                    reason,
                }
            })?;
            if descriptors_by_key
                .insert(key.as_str().to_owned(), *constraints)
                .is_some()
            {
                return Err(WorkspaceFileError::DuplicatePanelDescriptor {
                    key: key.as_str().to_owned(),
                });
            }
        }

        self.layout.canvas.validate().map_err(|reason| {
            WorkspaceFileError::InvalidRuntimeLayout(format!(
                "invalid canvas state: {reason:?}"
            ))
        })?;

        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let mut panel_instances = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut dock_areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let mut main_areas = [None, None];
        let mut floating_groups = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();

        for (side, root) in [
            (DockSide::Left, self.layout.left.as_ref()),
            (DockSide::Right, self.layout.right.as_ref()),
        ] {
            let Some(root) = root else {
                continue;
            };
            let root_id = restore_node(
                root,
                &descriptors_by_key,
                &mut nodes,
                &mut panel_instances,
            )?;
            let area_id = dock_areas.insert(DockArea::main(side, root_id));
            let index = match side {
                DockSide::Left => 0,
                DockSide::Right => 1,
            };
            main_areas[index] = Some(area_id);
        }

        for group in &self.layout.floating_groups {
            let root = restore_node(
                &group.root,
                &descriptors_by_key,
                &mut nodes,
                &mut panel_instances,
            )?;
            let area_id = dock_areas.insert(DockArea::floating(root));
            floating_groups.insert(FloatingGroup::new(area_id, group.bounds, group.z_order));
        }

        let layout = Layout::from_workspace_parts(
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            self.layout.canvas,
            self.layout.profile,
            self.layout.chrome,
        );
        layout
            .validate(metrics)
            .map_err(WorkspaceFileError::InvalidLayout)?;
        Ok(layout)
    }

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

impl Layout {
    /// Creates a stable workspace DTO suitable for the public JSON boundary.
    pub fn to_workspace_file(&self) -> Result<WorkspaceFile, WorkspaceFileError> {
        WorkspaceFile::from_layout(self)
    }
}

fn node_to_workspace(
    layout: &Layout,
    node_id: NodeId,
) -> Result<WorkspaceNode, WorkspaceFileError> {
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
                .map(|child| node_to_workspace(layout, *child))
                .collect::<Result<_, _>>()?,
            ratios: split.ratios.clone(),
        }),
        Node::Tabs(tabs) => Ok(WorkspaceNode::Tabs {
            children: tabs
                .children
                .iter()
                .map(|child| node_to_workspace(layout, *child))
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
                .map(|group| node_to_workspace(layout, *group))
                .collect::<Result<_, _>>()?,
            group_ratios: column.group_ratios.clone(),
            last_expanded_width: column.last_expanded_width,
        }),
    }
}

fn restore_node(
    workspace_node: &WorkspaceNode,
    descriptors: &HashMap<String, PanelConstraints>,
    nodes: &mut SlotMap<NodeId, Node>,
    panel_instances: &mut SlotMap<PanelId, PanelInstance>,
) -> Result<NodeId, WorkspaceFileError> {
    let node = match workspace_node {
        WorkspaceNode::Split {
            axis,
            children,
            ratios,
        } => {
            let child_ids = children
                .iter()
                .map(|child| restore_node(child, descriptors, nodes, panel_instances))
                .collect::<Result<_, _>>()?;
            Node::Split(Split {
                axis: *axis,
                children: child_ids,
                ratios: ratios.clone(),
            })
        }
        WorkspaceNode::Tabs {
            children,
            active_index,
            collapsed,
        } => {
            let child_ids = children
                .iter()
                .map(|child| restore_node(child, descriptors, nodes, panel_instances))
                .collect::<Result<_, _>>()?;
            Node::Tabs(Tabs {
                children: child_ids,
                active_index: *active_index,
                collapsed: *collapsed,
            })
        }
        WorkspaceNode::Panel { key, state } => {
            let constraints = descriptors.get(key.as_str()).copied().ok_or_else(|| {
                WorkspaceFileError::MissingPanelDescriptor {
                    key: key.as_str().to_owned(),
                }
            })?;
            let panel_id = panel_instances.insert(PanelInstance::with_state(
                key.clone(),
                constraints,
                *state,
            ));
            Node::Panel(Panel::new(panel_id))
        }
        WorkspaceNode::Column {
            width,
            mode,
            groups,
            group_ratios,
            last_expanded_width,
        } => {
            let group_ids = groups
                .iter()
                .map(|group| restore_node(group, descriptors, nodes, panel_instances))
                .collect::<Result<_, _>>()?;
            Node::Column(Column {
                width: *width,
                mode: *mode,
                groups: group_ids,
                group_ratios: group_ratios.clone(),
                last_expanded_width: *last_expanded_width,
            })
        }
    };
    Ok(nodes.insert(node))
}
