use std::collections::{BTreeSet, HashMap};

use slotmap::SlotMap;

use crate::{
    Column, ComputeRectsError, DockArea, DockAreaId, DockMetrics, DockSide, FloatingGroup,
    FloatingGroupId, Layout, Node, NodeId, Panel, PanelConstraints, PanelId,
    PanelInstance, PanelKey, Size, Split, Tabs, WorkspaceDiagnostic, WorkspaceFileError,
    WorkspaceFloatingGroup, WorkspaceLayout, WorkspaceLoadOptions, WorkspaceLoadResult,
    WorkspaceNode, layout::WorkspaceLayoutParts,
};

/// Marker identifying this file as a Roadmap Docking workspace.
pub const WORKSPACE_FORMAT: &str = "roadmap-docking-workspace";

/// Current public workspace JSON schema version.
pub const WORKSPACE_VERSION: u32 = 2;

/// Maximum accepted JSON workspace file size (16 MiB).
const MAX_WORKSPACE_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Maximum number of tree nodes restored from one workspace (including floating trees).
const MAX_WORKSPACE_NODES: usize = 100_000;
/// Maximum nesting depth accepted by workspace import/export, counting the root as level 1.
const MAX_WORKSPACE_TREE_DEPTH: usize = 256;

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

    /// Exports a validated Layout using the active metric profile.
    ///
    /// The structural export runs first so malformed or excessively deep runtime
    /// trees are rejected before recursive model validation.
    pub fn from_layout(
        layout: &Layout,
        metrics: &DockMetrics,
    ) -> Result<Self, WorkspaceFileError> {
        let file = Self::export_layout_unchecked(layout)?;
        layout
            .validate(metrics)
            .map_err(WorkspaceFileError::InvalidLayout)?;
        Ok(file)
    }

    /// Export helper for test fixtures that deliberately need an invalid Layout.
    #[cfg(test)]
    pub(crate) fn from_layout_unchecked_for_test(
        layout: &Layout,
    ) -> Result<Self, WorkspaceFileError> {
        Self::export_layout_unchecked(layout)
    }

    fn export_layout_unchecked(layout: &Layout) -> Result<Self, WorkspaceFileError> {
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

    /// Parses a workspace file and migrates supported older versions.
    ///
    /// This convenience method returns the upgraded file. Use
    /// `from_json_with_diagnostics` or `load_json` when migration warnings
    /// must be presented to the user or logged.
    pub fn from_json(json: &str) -> Result<Self, WorkspaceFileError> {
        Self::from_json_with_diagnostics(json).map(|(file, _diagnostics)| file)
    }

    /// Parses the JSON envelope and returns any schema migration diagnostics.
    pub fn from_json_with_diagnostics(
        json: &str,
    ) -> Result<(Self, Vec<WorkspaceDiagnostic>), WorkspaceFileError> {
        if json.len() > MAX_WORKSPACE_JSON_BYTES {
            return Err(WorkspaceFileError::Json(format!(
                "workspace JSON exceeds the maximum size of {MAX_WORKSPACE_JSON_BYTES} bytes"
            )));
        }
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))?;

        let format = value
            .get("format")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| WorkspaceFileError::Json("missing string field 'format'".to_owned()))?;
        if format != WORKSPACE_FORMAT {
            return Err(WorkspaceFileError::UnsupportedFormat {
                found: format.to_owned(),
            });
        }

        let version_value = value
            .get("version")
            .ok_or_else(|| WorkspaceFileError::Json("missing field 'version'".to_owned()))?;
        let version_u64 = version_value.as_u64().ok_or_else(|| {
            WorkspaceFileError::Json("field 'version' must be a non-negative integer".to_owned())
        })?;
        let version = u32::try_from(version_u64).map_err(|_| {
            WorkspaceFileError::Json(
                "workspace version is outside the supported integer range".to_owned(),
            )
        })?;
        if !matches!(version, 1 | WORKSPACE_VERSION) {
            return Err(WorkspaceFileError::UnsupportedVersion {
                found: version,
                supported: WORKSPACE_VERSION,
            });
        }

        // Version 1 and 2 share the same structural DTO. The explicit v1
        // migration establishes the compatibility seam for future shape changes.
        let mut file: Self = serde_json::from_value(value)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))?;
        let mut diagnostics = Vec::new();
        if file.version == 1 {
            file = migrate_v1_to_v2(file);
            diagnostics.push(WorkspaceDiagnostic::SchemaMigrated {
                from: 1,
                to: WORKSPACE_VERSION,
            });
        }
        file.validate_header()?;
        Ok((file, diagnostics))
    }

    /// Encodes this workspace as the canonical pretty-printed JSON file.
    pub fn to_json_pretty(&self) -> Result<String, WorkspaceFileError> {
        self.validate_header()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| WorkspaceFileError::Json(error.to_string()))
    }

    /// Restores a validated layout, preserving unknown panel kinds as placeholders.
    ///
    /// Without load options, floating bounds are left unchanged because Core
    /// cannot infer the active monitor or the current client region.
    pub fn into_layout(
        self,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
    ) -> Result<WorkspaceLoadResult, WorkspaceFileError> {
        self.into_layout_with_options(descriptors, metrics, WorkspaceLoadOptions::default())
    }

    /// Restores a layout, optionally clamping floating bounds to a client region.
    pub fn into_layout_with_options(
        self,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
        options: WorkspaceLoadOptions,
    ) -> Result<WorkspaceLoadResult, WorkspaceFileError> {
        self.into_layout_with_diagnostics(descriptors, metrics, options, Vec::new())
    }

    /// Full load pipeline: parse, migrate, rebuild fresh arenas, normalize safely,
    /// and return diagnostics in a single report.
    pub fn load_json(
        json: &str,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
        options: WorkspaceLoadOptions,
    ) -> Result<WorkspaceLoadResult, WorkspaceFileError> {
        let (file, diagnostics) = Self::from_json_with_diagnostics(json)?;
        file.into_layout_with_diagnostics(descriptors, metrics, options, diagnostics)
    }

    fn into_layout_with_diagnostics(
        mut self,
        descriptors: &[(PanelKey, PanelConstraints)],
        metrics: &DockMetrics,
        options: WorkspaceLoadOptions,
        mut diagnostics: Vec<WorkspaceDiagnostic>,
    ) -> Result<WorkspaceLoadResult, WorkspaceFileError> {
        // DTOs can be constructed directly in Rust, bypassing serde_json's
        // recursion limit. Check limits iteratively before migration/restore,
        // both of which recurse through the nested workspace tree.
        validate_workspace_tree_limits(&self.layout)?;
        if self.version == 1 {
            self = migrate_v1_to_v2(self);
            diagnostics.push(WorkspaceDiagnostic::SchemaMigrated {
                from: 1,
                to: WORKSPACE_VERSION,
            });
        }
        self.validate_header()?;
        metrics
            .validate()
            .map_err(WorkspaceFileError::InvalidMetrics)?;

        let visible_region = if let Some(region) = options.visible_client_region {
            // Rect owns the edge-representability contract: this rejects
            // positive dimensions that round back to their origins as well as
            // empty client regions, preserving the typed geometry error.
            region
                .validate_non_empty()
                .map_err(WorkspaceFileError::InvalidVisibleClientRegion)?;
            Some(region)
        } else {
            None
        };

        // Preserve the typed floating-bounds error and the pre-normalization
        // z-order index even though tree normalization now runs before clamping.
        if visible_region.is_some() {
            let mut ordered_floating = self.layout.floating_groups.iter().enumerate().collect::<Vec<_>>();
            ordered_floating.sort_by_key(|(_, group)| group.z_order);
            for (index, group) in ordered_floating.into_iter() {
                group
                    .bounds
                    .validate_non_empty()
                    .map_err(|reason| WorkspaceFileError::InvalidFloatingBounds {
                        index,
                        reason,
                    })?;
            }
        }

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

        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let mut panel_instances = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut dock_areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let mut main_areas = [None, None];
        let mut floating_groups = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
        let mut floating_group_json_indices = HashMap::<FloatingGroupId, usize>::new();
        let mut unknown_panel_keys = BTreeSet::new();

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
                metrics,
                &mut unknown_panel_keys,
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

        for (json_index, group) in self.layout.floating_groups.iter().enumerate() {
            let root = restore_node(
                &group.root,
                &descriptors_by_key,
                metrics,
                &mut unknown_panel_keys,
                &mut nodes,
                &mut panel_instances,
            )?;
            let area_id = dock_areas.insert(DockArea::floating(root));
            let floating_id =
                floating_groups.insert(FloatingGroup::new(area_id, group.bounds, group.z_order));
            floating_group_json_indices.insert(floating_id, json_index);
        }

        diagnostics.extend(
            unknown_panel_keys
                .into_iter()
                .map(|key| WorkspaceDiagnostic::UnknownPanelKeyPreserved { key }),
        );

        let mut layout = Layout::from_workspace_parts(WorkspaceLayoutParts {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            canvas: self.layout.canvas,
            profile: self.layout.profile,
            chrome: self.layout.chrome,
        });

        match layout.normalize_for_workspace_load(metrics) {
            Ok(Some(issues)) => diagnostics.push(WorkspaceDiagnostic::LayoutNormalized { issues }),
            Ok(None) => {}
            Err(issues) => return Err(WorkspaceFileError::InvalidLayout(issues)),
        }

        if let Some(region) = visible_region {
            let changes = layout
                .clamp_floating_groups_for_workspace_load(region, metrics, &floating_group_json_indices)
                .map_err(|(index, error)| match error {
                    ComputeRectsError::InsufficientSpace {
                        axis,
                        available,
                        required,
                    } => WorkspaceFileError::FloatingBoundsInsufficientSpace {
                        index,
                        axis,
                        available,
                        required,
                    },
                    ComputeRectsError::Geometry(reason) => {
                        WorkspaceFileError::InvalidFloatingBounds { index, reason }
                    }
                    ComputeRectsError::InvalidLayout => {
                        WorkspaceFileError::InvalidRuntimeLayout(format!(
                            "cannot clamp floating group {index}: its layout references are invalid"
                        ))
                    }
                    other => WorkspaceFileError::InvalidRuntimeLayout(format!(
                        "cannot clamp floating group {index}: {other}"
                    )),
                })?;
            diagnostics.extend(changes.into_iter().map(|(index, original, clamped)| {
                WorkspaceDiagnostic::FloatingBoundsClamped {
                    index,
                    original,
                    clamped,
                }
            }));
        }

        Ok(WorkspaceLoadResult {
            layout,
            diagnostics,
        })
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
    /// Creates a validated workspace DTO suitable for the public JSON boundary.
    pub fn to_workspace_file(
        &self,
        metrics: &DockMetrics,
    ) -> Result<WorkspaceFile, WorkspaceFileError> {
        WorkspaceFile::from_layout(self, metrics)
    }
}

fn validate_workspace_tree_limits(
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

fn migrate_v1_to_v2(mut file: WorkspaceFile) -> WorkspaceFile {
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

fn restore_node(
    workspace_node: &WorkspaceNode,
    descriptors: &HashMap<String, PanelConstraints>,
    metrics: &DockMetrics,
    unknown_panel_keys: &mut BTreeSet<String>,
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
                .map(|child| {
                    restore_node(
                        child,
                        descriptors,
                        metrics,
                        unknown_panel_keys,
                        nodes,
                        panel_instances,
                    )
                })
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
                .map(|child| {
                    restore_node(
                        child,
                        descriptors,
                        metrics,
                        unknown_panel_keys,
                        nodes,
                        panel_instances,
                    )
                })
                .collect::<Result<_, _>>()?;
            Node::Tabs(Tabs {
                children: child_ids,
                active_index: *active_index,
                collapsed: *collapsed,
            })
        }
        WorkspaceNode::Panel { key, state } => {
            let constraints = descriptors.get(key.as_str()).copied().unwrap_or_else(|| {
                unknown_panel_keys.insert(key.as_str().to_owned());
                placeholder_constraints(metrics)
            });
            let panel_id =
                panel_instances.insert(PanelInstance::with_state(key.clone(), constraints, *state));
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
                .map(|group| {
                    restore_node(
                        group,
                        descriptors,
                        metrics,
                        unknown_panel_keys,
                        nodes,
                        panel_instances,
                    )
                })
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

fn placeholder_constraints(metrics: &DockMetrics) -> PanelConstraints {
    // No descriptor means the former content minimum is unknowable. Use the
    // compact rail metrics for a conservative placeholder instead of imposing
    // the full expanded-column width as a panel-content minimum.
    let placeholder_size = Size::new(metrics.rail_width, metrics.collapsed_group_height);
    PanelConstraints::new(placeholder_size, placeholder_size, None, false)
}
