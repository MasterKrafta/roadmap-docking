use std::collections::{BTreeSet, HashMap};

use slotmap::SlotMap;

use crate::{
    Column, DockArea, DockAreaId, DockMetrics, DockSide, FloatingGroup, FloatingGroupId, Layout,
    Node, NodeId, Panel, PanelConstraints, PanelId, PanelInstance, PanelKey, Rect, Size, Split,
    Tabs, WorkspaceDiagnostic, WorkspaceFileError, WorkspaceFloatingGroup, WorkspaceLayout,
    WorkspaceLoadOptions, WorkspaceLoadResult, WorkspaceNode,
};

/// Marker identifying this file as a Roadmap Docking workspace.
pub const WORKSPACE_FORMAT: &str = "roadmap-docking-workspace";

/// Current public workspace JSON schema version.
pub const WORKSPACE_VERSION: u32 = 2;

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

        let version_u64 = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| WorkspaceFileError::Json("missing non-negative integer field 'version'".to_owned()))?;
        let version = u32::try_from(version_u64).map_err(|_| {
            WorkspaceFileError::Json("workspace version is outside the supported integer range".to_owned())
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
            region
                .validate_non_empty()
                .map_err(WorkspaceFileError::InvalidVisibleClientRegion)?;
            Some(region)
        } else {
            None
        };

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

        if let Some(region) = visible_region {
            for (index, group) in self.layout.floating_groups.iter_mut().enumerate() {
                let original = group.bounds;
                let clamped = clamp_floating_bounds(original, region, index)?;
                if original != clamped {
                    group.bounds = clamped;
                    diagnostics.push(WorkspaceDiagnostic::FloatingBoundsClamped {
                        index,
                        original,
                        clamped,
                    });
                }
            }
        }

        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let mut panel_instances = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut dock_areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let mut main_areas = [None, None];
        let mut floating_groups = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
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

        for group in &self.layout.floating_groups {
            let root = restore_node(
                &group.root,
                &descriptors_by_key,
                metrics,
                &mut unknown_panel_keys,
                &mut nodes,
                &mut panel_instances,
            )?;
            let area_id = dock_areas.insert(DockArea::floating(root));
            floating_groups.insert(FloatingGroup::new(area_id, group.bounds, group.z_order));
        }

        diagnostics.extend(
            unknown_panel_keys
                .into_iter()
                .map(|key| WorkspaceDiagnostic::UnknownPanelKeyPreserved { key }),
        );

        let mut layout = Layout::from_workspace_parts(
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            self.layout.canvas,
            self.layout.profile,
            self.layout.chrome,
        );

        match layout.normalize_for_workspace_load(metrics) {
            Ok(Some(issues)) => diagnostics.push(WorkspaceDiagnostic::LayoutNormalized { issues }),
            Ok(None) => {}
            Err(issues) => return Err(WorkspaceFileError::InvalidLayout(issues)),
        }

        Ok(WorkspaceLoadResult { layout, diagnostics })
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

fn migrate_v1_to_v2(mut file: WorkspaceFile) -> WorkspaceFile {
    // The first versioned schema used this same layout DTO. Version 2 keeps the
    // structural shape while making migration explicit and diagnosable.
    file.version = WORKSPACE_VERSION;
    file
}

fn clamp_floating_bounds(
    bounds: Rect,
    visible_region: Rect,
    index: usize,
) -> Result<Rect, WorkspaceFileError> {
    bounds
        .validate_non_empty()
        .map_err(|reason| WorkspaceFileError::InvalidFloatingBounds { index, reason })?;
    let region_right = visible_region
        .checked_right()
        .map_err(WorkspaceFileError::InvalidVisibleClientRegion)?;
    let region_bottom = visible_region
        .checked_bottom()
        .map_err(WorkspaceFileError::InvalidVisibleClientRegion)?;

    // When a stored floating window is larger than the current client region,
    // shrink it to fit and report the recovery through FloatingBoundsClamped.
    let width = bounds.size.width.min(visible_region.size.width);
    let height = bounds.size.height.min(visible_region.size.height);
    let max_x = region_right - width;
    let max_y = region_bottom - height;
    let x = bounds.origin.x.clamp(visible_region.origin.x, max_x);
    let y = bounds.origin.y.clamp(visible_region.origin.y, max_y);
    let clamped = Rect::new(crate::Point::new(x, y), Size::new(width, height));
    clamped
        .validate_non_empty()
        .map_err(|reason| WorkspaceFileError::InvalidFloatingBounds { index, reason })?;
    Ok(clamped)
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
                        child, descriptors, metrics, unknown_panel_keys, nodes, panel_instances,
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
                        child, descriptors, metrics, unknown_panel_keys, nodes, panel_instances,
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
                .map(|group| {
                    restore_node(
                        group, descriptors, metrics, unknown_panel_keys, nodes, panel_instances,
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
    let placeholder_size = Size::new(metrics.min_column_width, metrics.min_group_height);
    PanelConstraints::new(placeholder_size, placeholder_size, None, false)
}
