use std::collections::{BTreeSet, HashMap};

use slotmap::SlotMap;

use super::WorkspaceFile;
use super::schema::{migrate_v1_to_v2, validate_workspace_tree_limits};
use crate::{
    Column, ComputeRectsError, DockArea, DockAreaId, DockMetrics, DockSide, FloatingGroup,
    FloatingGroupId, Layout, Node, NodeId, Panel, PanelConstraints, PanelId, PanelInstance,
    PanelKey, Size, Split, Tabs, WorkspaceDiagnostic, WorkspaceFileError, WorkspaceLayout,
    WorkspaceLoadOptions, WorkspaceLoadResult, WorkspaceNode, layout::WorkspaceLayoutParts,
};

impl WorkspaceFile {
    pub(super) fn into_layout_with_diagnostics(
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
            let mut ordered_floating = self
                .layout
                .floating_groups
                .iter()
                .enumerate()
                .collect::<Vec<_>>();
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
                .clamp_floating_groups_for_workspace_load(
                    region,
                    metrics,
                    &floating_group_json_indices,
                )
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
