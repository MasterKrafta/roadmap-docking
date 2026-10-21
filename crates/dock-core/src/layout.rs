use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    command_application::CommandApplication, Axis, CanvasState, ChromeMode, ColumnMode,
    CommandError, CommandOutcome, CommandValue, DockArea, DockAreaId, DockAreaKind, DockCommand,
    DockMetrics, DockSide, DropTarget, DropZone, FloatingGroup, FloatingGroupId, FlyoutState,
    GeometryError, LastDockLocation, Node, NodeId, PanelId, PanelInstance, PanelKey, Point, Profile,
    Rect, Size, ValidationError,
};

const RATIO_TOLERANCE: f32 = 1.0e-4;
const PIXEL_TOLERANCE: f32 = 1.0e-3;

/// Complete UI-independent docking state.
///
/// Layout is the sole owner of runtime arenas. Public access is read-only;
/// external edits are performed only through Layout::apply and typed DockCommand
/// values. Flyout state is transient and kept separately from the persistent tree.
#[derive(Clone, Debug)]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panel_instances: SlotMap<PanelId, PanelInstance>,
    dock_areas: SlotMap<DockAreaId, DockArea>,
    main_areas: [Option<DockAreaId>; 2],
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    last_dock_locations: HashMap<PanelKey, LastDockLocation>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
    chrome: ChromeMode,
    flyout: Option<FlyoutState>,
}

impl Layout {
    /// Creates an empty layout with an explicit profile and a mandatory canvas.
    pub fn new(profile: Profile) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panel_instances: SlotMap::with_key(),
            dock_areas: SlotMap::with_key(),
            main_areas: [None, None],
            floating_groups: SlotMap::with_key(),
            last_dock_locations: HashMap::new(),
            canvas: CanvasState::default(),
            profile,
            version: 0,
            chrome: ChromeMode::Visible,
            flyout: None,
        }
    }

    pub const fn profile(&self) -> Profile {
        self.profile
    }

    pub const fn version(&self) -> u64 {
        self.version
    }

    pub const fn chrome(&self) -> ChromeMode {
        self.chrome
    }

    pub const fn canvas(&self) -> &CanvasState {
        &self.canvas
    }

    pub const fn flyout(&self) -> Option<FlyoutState> {
        self.flyout
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn panel_instance(&self, id: PanelId) -> Option<&PanelInstance> {
        self.panel_instances.get(id)
    }

    /// Returns the last known main-dock placement for a panel key in this
    /// runtime session. This preference is intentionally not workspace state.
    pub fn last_dock_location(&self, key: &PanelKey) -> Option<LastDockLocation> {
        self.last_dock_locations.get(key).copied()
    }

    pub fn dock_area(&self, id: DockAreaId) -> Option<&DockArea> {
        self.dock_areas.get(id)
    }

    pub fn floating_group(&self, id: FloatingGroupId) -> Option<&FloatingGroup> {
        self.floating_groups.get(id)
    }

    /// Returns the main area for a side, if one exists.
    pub fn main_area(&self, side: DockSide) -> Option<&DockArea> {
        let index = side_index(side);
        self.main_areas[index].and_then(|id| self.dock_areas.get(id))
    }

    /// Read-only arena views for validation, rendering adapters, and inspection.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter()
    }

    pub fn panel_instances(&self) -> impl Iterator<Item = (PanelId, &PanelInstance)> {
        self.panel_instances.iter()
    }

    pub fn dock_areas(&self) -> impl Iterator<Item = (DockAreaId, &DockArea)> {
        self.dock_areas.iter()
    }

    pub fn floating_groups(&self) -> impl Iterator<Item = (FloatingGroupId, &FloatingGroup)> {
        self.floating_groups.iter()
    }

    /// Returns floating windows in paint order: lowest z_order first (back-to-front).
    ///
    /// Render adapters should draw windows in this order. Valid layouts require
    /// unique z_order values, so stacking order is unambiguous.
    pub fn floating_groups_in_paint_order(&self) -> Vec<(FloatingGroupId, &FloatingGroup)> {
        let mut groups: Vec<_> = self.floating_groups.iter().collect();
        groups.sort_by_key(|(_, group)| group.z_order);
        groups
    }

    /// Returns floating windows in hit-test order: highest z_order first.
    ///
    /// Input adapters should test the first returned window before the rest.
    pub fn floating_groups_in_hit_test_order(&self) -> Vec<(FloatingGroupId, &FloatingGroup)> {
        let mut groups: Vec<_> = self.floating_groups.iter().collect();
        groups.sort_by(|(_, left), (_, right)| right.z_order.cmp(&left.z_order));
        groups
    }


    /// Applies one command transactionally. All edits happen on a candidate copy;
    /// the live layout is replaced only after normalization and validation succeed.
    pub fn apply(
        &mut self,
        command: DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandOutcome, CommandError> {
        tracing::debug!(?command, version = self.version, "applying layout command");
        let prior_version = self.version;
        let result = self.apply_transaction(&command, metrics);
        match &result {
            Ok(outcome) if outcome.new_version != prior_version => {
                tracing::info!(
                    ?command,
                    old_version = prior_version,
                    new_version = outcome.new_version,
                    "layout command committed"
                );
            }
            Ok(_) => {
                tracing::debug!(?command, version = prior_version, "layout command accepted without version change");
            }
            Err(error) => {
                tracing::warn!(?command, ?error, version = prior_version, "layout command rejected");
            }
        }
        result
    }

    fn apply_transaction(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandOutcome, CommandError> {
        self.validate(metrics).map_err(|errors| CommandError::InvalidLayout { errors })?;

        let mut candidate = self.clone();
        let applied = candidate.apply_command(command, metrics)?;
        candidate.normalize_in_place(metrics).map_err(|errors| CommandError::InvalidLayout { errors })?;

        if candidate.same_state(self) {
            return Ok(CommandOutcome {
                changed_nodes: Vec::new(),
                new_version: self.version,
                actual_value: applied.actual_value,
                inverse: None,
            });
        }

        if !applied.persistent {
            candidate.version = self.version;
            *self = candidate;
            return Ok(CommandOutcome {
                changed_nodes: Vec::new(),
                new_version: self.version,
                actual_value: applied.actual_value,
                inverse: None,
            });
        }

        let next_version = self.version.checked_add(1).ok_or(CommandError::InvalidValue {
            field: "version",
            reason: "layout version overflow",
        })?;
        let changed_nodes = self.changed_node_ids(&candidate);
        candidate.version = next_version;
        *self = candidate;

        Ok(CommandOutcome {
            changed_nodes,
            new_version: next_version,
            actual_value: applied.actual_value,
            inverse: applied.inverse,
        })
    }

    fn apply_command(
        &mut self,
        command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        match command {
            DockCommand::OpenPanel { key, constraints, target } => {
                constraints.validate().map_err(|error| CommandError::InvalidValue {
                    field: "constraints",
                    reason: geometry_error_reason(error),
                })?;

                // Re-opening an existing singleton activates it instead of toggling
                // it closed. Its previous placement is authoritative, so a fresh
                // target is not needed and may no longer exist.
                if let Some((existing_id, _)) = self.panel_instances.iter().find(|(_, instance)| {
                    instance.key == *key && (constraints.singleton || instance.constraints.singleton)
                }) {
                    let panel_node = self.find_panel_node(existing_id).ok_or(
                        CommandError::MissingPanel { panel: existing_id },
                    )?;
                    let Some((group, _)) = self.tab_parent_index(panel_node) else {
                        // A single Panel may be the root of a Generic floating/main
                        // area. It is already open and visible; there is no tab index
                        // to change, so treat this as an accepted no-op.
                        return Ok(CommandApplication::persistent());
                    };
                    let tabs = match self.nodes.get(group) {
                        Some(Node::Tabs(tabs)) => tabs,
                        _ => return Err(CommandError::InvalidTarget {
                            reason: "existing singleton panel parent is not a Tabs node",
                        }),
                    };
                    let previous_active = tabs
                        .children
                        .get(tabs.active_index)
                        .and_then(|id| match self.nodes.get(*id) {
                            Some(Node::Panel(panel)) => Some(panel.panel_id),
                            _ => None,
                        });
                    let Some(index) = tabs.children.iter().position(|id| *id == panel_node) else {
                        return Err(CommandError::InvalidTarget {
                            reason: "existing singleton panel is not a child of its Tabs node",
                        });
                    };
                    if index == tabs.active_index {
                        return Ok(CommandApplication::persistent());
                    }
                    if let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) {
                        tabs.active_index = index;
                    }
                    let inverse = previous_active.map(|panel| DockCommand::ActivateTab { group, panel });
                    return Ok(CommandApplication {
                        actual_value: None,
                        inverse,
                        persistent: true,
                    });
                }

                let resolved_target = match *target {
                    Some(target) => target,
                    None => self.resolve_open_target(key, metrics)?,
                };
                self.validate_drop_target(resolved_target, metrics)?;
                let panel_id = self.panel_instances.insert(PanelInstance::new(key.clone(), *constraints));
                let panel_node = self.nodes.insert(Node::Panel(crate::Panel::new(panel_id)));
                self.insert_node_for_target(panel_node, resolved_target, metrics, Some(0.5))?;
                self.remember_panel_dock_location(panel_node);
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::ClosePanel { panel: panel_id }),
                    persistent: true,
                })
            }
            DockCommand::ClosePanel { panel } => {
                if !self.panel_instances.contains_key(*panel) {
                    return Err(CommandError::MissingPanel { panel: *panel });
                }
                let node = self.find_panel_node(*panel).ok_or(CommandError::MissingPanel { panel: *panel })?;
                self.remember_panel_dock_location(node);
                self.detach_node(node)?;
                self.remove_subtree(node);
                Ok(CommandApplication::persistent())
            }
            DockCommand::CloseGroup { group } => {
                match self.nodes.get(*group) {
                    Some(Node::Tabs(_)) => {}
                    Some(_) => return Err(CommandError::InvalidTarget {
                        reason: "CloseGroup target must be a Tabs node",
                    }),
                    None => return Err(CommandError::MissingNode { node: *group }),
                }
                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                self.remove_subtree(*group);
                Ok(CommandApplication::persistent())
            }
            DockCommand::ActivateTab { group, panel } => {
                let tabs = match self.nodes.get(*group) {
                    Some(Node::Tabs(tabs)) => tabs.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget {
                        reason: "ActivateTab target must be a Tabs node",
                    }),
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                let panel_node = self.find_panel_node(*panel).ok_or(CommandError::MissingPanel { panel: *panel })?;
                let Some(index) = tabs.children.iter().position(|id| *id == panel_node) else {
                    return Err(CommandError::InvalidTarget {
                        reason: "panel is not a tab in the specified group",
                    });
                };
                let previous = tabs.children.get(tabs.active_index).and_then(|id| match self.nodes.get(*id) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                });
                if index == tabs.active_index {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Tabs(tabs)) = self.nodes.get_mut(*group) {
                    tabs.active_index = index;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: previous.map(|panel| DockCommand::ActivateTab { group: *group, panel }),
                    persistent: true,
                })
            }
            DockCommand::ReorderTab { group, panel, to_index } => {
                let tabs = match self.nodes.get(*group) {
                    Some(Node::Tabs(tabs)) => tabs.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget {
                        reason: "ReorderTab target must be a Tabs node",
                    }),
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                let panel_node = self.find_panel_node(*panel).ok_or(CommandError::MissingPanel { panel: *panel })?;
                let Some(from_index) = tabs.children.iter().position(|id| *id == panel_node) else {
                    return Err(CommandError::InvalidTarget {
                        reason: "panel is not a tab in the specified group",
                    });
                };
                if *to_index >= tabs.children.len() {
                    return Err(CommandError::InvalidValue {
                        field: "to_index",
                        reason: "tab index is outside the group",
                    });
                }
                if from_index == *to_index {
                    return Ok(CommandApplication::persistent());
                }
                let active_node = tabs.children.get(tabs.active_index).copied();
                let Some(Node::Tabs(current)) = self.nodes.get_mut(*group) else {
                    return Err(CommandError::InvalidTarget { reason: "Tabs node changed unexpectedly" });
                };
                let moved = current.children.remove(from_index);
                current.children.insert(*to_index, moved);
                if let Some(active_node) = active_node {
                    if let Some(active_index) = current.children.iter().position(|id| *id == active_node) {
                        current.active_index = active_index;
                    }
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::ReorderTab {
                        group: *group,
                        panel: *panel,
                        to_index: from_index,
                    }),
                    persistent: true,
                })
            }
            DockCommand::MovePanel { panel, target } => {
                let panel_node = self.find_panel_node(*panel).ok_or(CommandError::MissingPanel { panel: *panel })?;
                self.validate_drop_target(*target, metrics)?;
                if let (Some((source_group, source_index)), DropTarget::Tabs { group: target_group, index }) =
                    (self.tab_parent_index(panel_node), *target)
                {
                    if source_group == target_group
                        && (index == source_index || index == source_index + 1)
                    {
                        return Ok(CommandApplication::persistent());
                    }
                    if source_group == target_group {
                        let tabs = match self.nodes.get_mut(source_group) {
                            Some(Node::Tabs(tabs)) => tabs,
                            _ => return Err(CommandError::InvalidTarget { reason: "panel parent is not a Tabs node" }),
                        };
                        let active_node = tabs.children.get(tabs.active_index).copied();
                        let moved = tabs.children.remove(source_index);
                        let insert_index = if index > source_index { index - 1 } else { index };
                        tabs.children.insert(insert_index.min(tabs.children.len()), moved);
                        if let Some(active_node) = active_node {
                            if let Some(active_index) = tabs.children.iter().position(|id| *id == active_node) {
                                tabs.active_index = active_index;
                            }
                        }
                        return Ok(CommandApplication {
                            actual_value: None,
                            inverse: Some(DockCommand::ReorderTab {
                                group: source_group,
                                panel: *panel,
                                to_index: source_index,
                            }),
                            persistent: true,
                        });
                    }
                }
                let inverse = self.tab_parent_index(panel_node).and_then(|(group, index)| {
                    let source_group_has_other_tabs = matches!(
                        self.nodes.get(group),
                        Some(Node::Tabs(tabs)) if tabs.children.len() > 1
                    );
                    source_group_has_other_tabs.then_some(DockCommand::MovePanel {
                        panel: *panel,
                        target: DropTarget::Tabs { group, index },
                    })
                });
                self.remember_panel_dock_location(panel_node);
                self.detach_node(panel_node)?;
                self.insert_node_for_target(panel_node, *target, metrics, Some(0.5))?;
                self.remember_panel_dock_location(panel_node);
                Ok(CommandApplication { actual_value: None, inverse, persistent: true })
            }
            DockCommand::MoveGroup { group, column, index } => {
                let _ = match self.nodes.get(*group) {
                    Some(Node::Tabs(_)) => {}
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "MoveGroup source must be a Tabs node" }),
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                let target_groups = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.groups.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "MoveGroup target must be a Column node" }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if *index > target_groups.len() {
                    return Err(CommandError::InvalidValue { field: "index", reason: "group insertion index is outside the column" });
                }
                let source_parent = self.find_parent(*group);
                if source_parent == Some(*column) {
                    let source_index = target_groups.iter().position(|id| id == group);
                    if let Some(source_index) = source_index {
                        if *index == source_index || *index == source_index + 1 {
                            return Ok(CommandApplication::persistent());
                        }
                        let adjusted_index = if *index > source_index { *index - 1 } else { *index };
                        let inverse_index = if source_index > adjusted_index {
                            source_index + 1
                        } else {
                            source_index
                        };
                        let inverse = Some(DockCommand::MoveGroup {
                            group: *group,
                            column: *column,
                            index: inverse_index,
                        });
                        let Some(Node::Column(target_column)) = self.nodes.get_mut(*column) else {
                            return Err(CommandError::InvalidTarget {
                                reason: "MoveGroup target changed unexpectedly",
                            });
                        };
                        let moved_group = target_column.groups.remove(source_index);
                        let moved_ratio = target_column.group_ratios.remove(source_index);
                        target_column.groups.insert(adjusted_index, moved_group);
                        target_column.group_ratios.insert(adjusted_index, moved_ratio);
                        self.remember_subtree_dock_locations(*group);
                        return Ok(CommandApplication { actual_value: None, inverse, persistent: true });
                    }
                }
                let source_column = source_parent.and_then(|parent| match self.nodes.get(parent) {
                    Some(Node::Column(column)) if column.groups.len() > 1 => {
                        column.groups.iter().position(|id| *id == *group).map(|index| (parent, index))
                    }
                    _ => None,
                });
                let inverse = source_column.map(|(source_column, source_index)| {
                    DockCommand::MoveGroup { group: *group, column: source_column, index: source_index }
                });
                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                let new_len = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.groups.len(),
                    _ => return Err(CommandError::InvalidTarget { reason: "MoveGroup target changed unexpectedly" }),
                };
                self.insert_group_at_column(*column, *group, (*index).min(new_len), None)?;
                self.remember_subtree_dock_locations(*group);
                Ok(CommandApplication { actual_value: None, inverse, persistent: true })
            }
            DockCommand::SplitGroup { group, target_group, zone, ratio } => {
                if !ratio.is_finite() || *ratio <= 0.0 || *ratio >= 1.0 {
                    return Err(CommandError::InvalidValue {
                        field: "ratio",
                        reason: "split ratio must be finite and strictly between 0 and 1",
                    });
                }
                if !matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
                    return Err(if self.nodes.contains_key(*group) {
                        CommandError::InvalidTarget { reason: "SplitGroup source must be a Tabs node" }
                    } else {
                        CommandError::MissingNode { node: *group }
                    });
                }
                if !matches!(self.nodes.get(*target_group), Some(Node::Tabs(_))) {
                    return Err(if self.nodes.contains_key(*target_group) {
                        CommandError::InvalidTarget { reason: "SplitGroup target must be a Tabs node" }
                    } else {
                        CommandError::MissingNode { node: *target_group }
                    });
                }
                self.validate_drop_target(DropTarget::GroupEdge { group: *target_group, zone: *zone }, metrics)?;
                if group == target_group {
                    return Ok(CommandApplication::persistent());
                }

                // A horizontal split that reproduces the existing neighboring
                // column placement is a no-op when the moving group is the
                // source column's only group. Do not replace the Column node or
                // perturb ratios for a drop that leaves the tree unchanged.
                if matches!(zone, DropZone::Left | DropZone::Right) {
                    if let (Some(source_column), Some(target_column)) = (
                        self.find_owner_column(*group),
                        self.find_owner_column(*target_group),
                    ) {
                        let source_is_single_group_column = matches!(
                            self.nodes.get(source_column),
                            Some(Node::Column(column))
                                if column.groups.len() == 1 && column.groups[0] == *group
                        );
                        if source_is_single_group_column {
                            if let (Some((source_side, source_index)), Some((target_side, target_index))) = (
                                self.main_column_position(source_column),
                                self.main_column_position(target_column),
                            ) {
                                let already_at_requested_edge = source_side == target_side
                                    && ((*zone == DropZone::Left && source_index + 1 == target_index)
                                        || (*zone == DropZone::Right && target_index + 1 == source_index));
                                if already_at_requested_edge {
                                    return Ok(CommandApplication::persistent());
                                }
                            }
                        }
                    }
                }

                if let Some(source_column) = self.find_owner_column(*group) {
                    if self.find_owner_column(*target_group) == Some(source_column) {
                        let groups = match self.nodes.get(source_column) {
                            Some(Node::Column(column)) => column.groups.clone(),
                            _ => Vec::new(),
                        };
                        let source_index = groups.iter().position(|id| id == group);
                        let target_index = groups.iter().position(|id| id == target_group);
                        if let (Some(source_index), Some(target_index)) = (source_index, target_index) {
                            if (*zone == DropZone::Top && source_index + 1 == target_index)
                                || (*zone == DropZone::Bottom && target_index + 1 == source_index)
                            {
                                return Ok(CommandApplication::persistent());
                            }
                        }
                    }
                }
                // A floating vertical stack has the same neighboring no-op
                // rule as a Column's ordered group list.
                if matches!(zone, DropZone::Top | DropZone::Bottom) {
                    if let (Some(source_parent), Some(target_parent)) = (
                        self.find_parent(*group),
                        self.find_parent(*target_group),
                    ) {
                        if source_parent == target_parent {
                            if let Some(Node::Split(split)) = self.nodes.get(source_parent) {
                                if split.axis == Axis::Y {
                                    let source_index = split.children.iter().position(|id| id == group);
                                    let target_index = split.children.iter().position(|id| id == target_group);
                                    if let (Some(source_index), Some(target_index)) =
                                        (source_index, target_index)
                                    {
                                        let already_at_requested_edge =
                                            (*zone == DropZone::Top && source_index + 1 == target_index)
                                                || (*zone == DropZone::Bottom
                                                    && target_index + 1 == source_index);
                                        if already_at_requested_edge {
                                            return Ok(CommandApplication::persistent());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let source_column = self.find_owner_column(*group);
                let source_index = source_column.and_then(|column| match self.nodes.get(column) {
                    Some(Node::Column(column_data)) if column_data.groups.len() > 1 => {
                        column_data.groups.iter().position(|id| id == group).map(|index| (column, index))
                    }
                    _ => None,
                });
                self.remember_subtree_dock_locations(*group);
                self.detach_node(*group)?;
                self.insert_group_edge(*group, *target_group, *zone, Some(*ratio), metrics)?;
                self.remember_subtree_dock_locations(*group);
                let inverse = source_index.and_then(|(column, original_index)| {
                    let restore_index = if self.find_owner_column(*group) == Some(column) {
                        match self.nodes.get(column) {
                            Some(Node::Column(column_data)) => column_data
                                .groups
                                .iter()
                                .position(|id| id == group)
                                .map(|current_index| {
                                    if original_index > current_index {
                                        original_index + 1
                                    } else {
                                        original_index
                                    }
                                }),
                            _ => None,
                        }
                    } else {
                        Some(original_index)
                    };
                    restore_index.map(|index| DockCommand::MoveGroup {
                        group: *group,
                        column,
                        index,
                    })
                });
                Ok(CommandApplication { actual_value: None, inverse, persistent: true })
            }
            DockCommand::ResizeGroups {
                column,
                boundary_index,
                delta,
                available_height,
            } => {
                if !delta.is_finite() {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "group boundary delta must be finite",
                    });
                }
                if !available_height.is_finite() || *available_height <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "available group height must be finite and positive",
                    });
                }
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget {
                        reason: "ResizeGroups target must be a Column node",
                    }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if *boundary_index >= current.groups.len().saturating_sub(1) {
                    return Err(CommandError::InvalidTarget {
                        reason: "group boundary index does not identify adjacent groups",
                    });
                }
                let left_group = current.groups[*boundary_index];
                let right_group = current.groups[*boundary_index + 1];
                let heights = self.group_heights_for_available_height(
                    *column,
                    *available_height,
                    metrics,
                )?;
                if self.group_is_collapsed(left_group) || self.group_is_collapsed(right_group) {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }

                let minimums = self.group_minimum_heights(*column, metrics)?;
                let left_index = *boundary_index;
                let right_index = left_index + 1;
                let lower_delta = minimums[left_index] - heights[left_index];
                let upper_delta = heights[right_index] - minimums[right_index];
                if lower_delta > upper_delta {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "available height cannot satisfy adjacent group minimums",
                    });
                }
                let requested_delta = (*delta).clamp(lower_delta, upper_delta);
                if requested_delta == 0.0 {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }

                if current.groups.iter().enumerate().any(|(index, group)| {
                    !self.group_is_collapsed(*group)
                        && heights[index] + PIXEL_TOLERANCE < minimums[index]
                }) {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "current group ratios place an expanded group below its minimum; reset ratios before resizing",
                    });
                }

                // Ratios are weights among expanded groups. Transfer weight only
                // between this pair so every non-adjacent group's ratio remains
                // unchanged and the pair keeps its combined weight.
                let expanded_weight_sum: f32 = current.groups.iter().enumerate()
                    .filter(|(_, group)| !self.group_is_collapsed(**group))
                    .map(|(index, _)| current.group_ratios[index])
                    .sum();
                let expanded_height: f32 = current.groups.iter().enumerate()
                    .filter(|(_, group)| !self.group_is_collapsed(**group))
                    .map(|(index, _)| heights[index])
                    .sum();
                if expanded_weight_sum <= f32::EPSILON || expanded_height <= 0.0 {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }

                let weight_delta = requested_delta * expanded_weight_sum / expanded_height;
                let mut new_ratios = current.group_ratios.clone();
                new_ratios[left_index] += weight_delta;
                new_ratios[right_index] -= weight_delta;
                if new_ratios[left_index] < 0.0 || new_ratios[right_index] < 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "constrained resize produced a negative group weight",
                    });
                }
                let final_heights = self.group_heights_for_ratios(
                    *column,
                    &new_ratios,
                    *available_height,
                    metrics,
                )?;
                let actual_delta = final_heights[left_index] - heights[left_index];
                if actual_delta == 0.0 || new_ratios == current.group_ratios {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::GroupBoundaryDelta(0.0)),
                        inverse: None,
                        persistent: true,
                    });
                }
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.group_ratios = new_ratios;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::GroupBoundaryDelta(actual_delta)),
                    inverse: Some(DockCommand::RestoreGroupRatios {
                        column: *column,
                        ratios: current.group_ratios,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeColumn { column, width } => {
                if !width.is_finite() || *width <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "width must be finite and positive",
                    });
                }
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget {
                        reason: "ResizeColumn target must be a Column node",
                    }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                let old_width = current.width;
                if let Some((side, column_index)) = self.main_column_position(*column) {
                    let main_columns = self.main_columns(side)?;
                    let canvas_adjacent = match side {
                        DockSide::Left => column_index + 1 == main_columns.len(),
                        DockSide::Right => column_index == 0,
                    };
                    if !canvas_adjacent {
                        return Err(CommandError::InvalidTarget {
                            reason: "interior main columns must be resized through ResizeColumnBoundary",
                        });
                    }
                }
                let min_width = self.minimum_column_width(*column, metrics)?;
                let panel_max = self.maximum_column_width(*column)?;
                let mut max_width = panel_max.unwrap_or(f32::MAX);

                // When the canvas has a measured remainder, resizing an outer
                // main-dock column consumes or returns that remainder. Preserve
                // the mandatory min_canvas_width rather than letting the UI
                // render a canvas narrower than Core's own invariant.
                if let Some((_, _)) = self.main_column_position(*column) {
                    if let Some(bounds) = self.canvas.last_known_bounds {
                        let room = (bounds.size.width - metrics.min_canvas_width).max(0.0);
                        let canvas_cap = old_width + room;
                        if canvas_cap.is_finite() {
                            max_width = max_width.min(canvas_cap);
                        }
                    }
                }
                if max_width < min_width {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "column minimum width exceeds its effective maximum width",
                    });
                }
                let actual_width = match current.mode {
                    ColumnMode::IconsOnly => metrics.rail_width,
                    ColumnMode::Expanded => width.clamp(min_width, max_width),
                };
                if !actual_width.is_finite() || actual_width <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "width",
                        reason: "effective width is not finite and positive",
                    });
                }
                let actual_delta = actual_width - old_width;
                if actual_delta != 0.0 {
                    if let Some(Node::Column(column)) = self.nodes.get_mut(*column) {
                        column.width = actual_width;
                    }
                    self.adjust_canvas_for_column_resize(*column, actual_delta, metrics)?;
                    self.sync_main_column_ratios(*column)?;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::ColumnWidth(actual_width)),
                    inverse: (actual_delta != 0.0).then_some(DockCommand::ResizeColumn {
                        column: *column,
                        width: old_width,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResizeColumnBoundary { side, boundary_index, delta } => {
                if !delta.is_finite() {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "column boundary delta must be finite",
                    });
                }
                let columns = self.main_columns(*side)?;
                if *boundary_index >= columns.len().saturating_sub(1) {
                    return Err(CommandError::InvalidTarget {
                        reason: "column boundary index does not identify adjacent columns",
                    });
                }
                let left_id = columns[*boundary_index];
                let right_id = columns[*boundary_index + 1];
                let left = match self.nodes.get(left_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => return Err(CommandError::InvalidTarget {
                        reason: "main dock contains a non-Column child",
                    }),
                };
                let right = match self.nodes.get(right_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => return Err(CommandError::InvalidTarget {
                        reason: "main dock contains a non-Column child",
                    }),
                };
                let actual_delta = if left.mode == ColumnMode::IconsOnly || right.mode == ColumnMode::IconsOnly {
                    0.0
                } else {
                    let left_min = self.minimum_column_width(left_id, metrics)?;
                    let right_min = self.minimum_column_width(right_id, metrics)?;
                    let left_max = self.maximum_column_width(left_id)?.unwrap_or(f32::INFINITY);
                    let right_max = self.maximum_column_width(right_id)?.unwrap_or(f32::INFINITY);
                    let lower_delta = (left_min - left.width).max(right.width - right_max);
                    let upper_delta = (right.width - right_min).min(left_max - left.width);
                    if lower_delta > upper_delta {
                        return Err(CommandError::InvalidValue {
                            field: "delta",
                            reason: "adjacent columns have incompatible min/max width constraints",
                        });
                    }
                    (*delta).clamp(lower_delta, upper_delta)
                };
                if actual_delta != 0.0 {
                    let new_left = left.width + actual_delta;
                    let new_right = right.width - actual_delta;
                    if !new_left.is_finite() || !new_right.is_finite() || new_left <= 0.0 || new_right <= 0.0 {
                        return Err(CommandError::InvalidValue {
                            field: "delta",
                            reason: "resized column width is not finite and positive",
                        });
                    }
                    if let Some(Node::Column(column)) = self.nodes.get_mut(left_id) {
                        column.width = new_left;
                    }
                    if let Some(Node::Column(column)) = self.nodes.get_mut(right_id) {
                        column.width = new_right;
                    }
                    self.sync_main_column_ratios(left_id)?;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::ColumnBoundaryDelta(actual_delta)),
                    inverse: (actual_delta != 0.0).then_some(DockCommand::ResizeColumnBoundary {
                        side: *side,
                        boundary_index: *boundary_index,
                        delta: -actual_delta,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResetColumnRatios { column, available_height } => {
                if !available_height.is_finite() || *available_height <= 0.0 {
                    return Err(CommandError::InvalidValue {
                        field: "available_height",
                        reason: "available group height must be finite and positive",
                    });
                }
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget {
                        reason: "ResetColumnRatios target must be a Column node",
                    }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                // Validate the supplied height against fixed collapsed strips
                // and expanded minimums even when the reset itself becomes a no-op.
                let _current_heights = self.group_heights_for_available_height(
                    *column,
                    *available_height,
                    metrics,
                )?;
                if current.groups.iter().all(|group| self.group_is_collapsed(*group)) {
                    return Ok(CommandApplication::persistent());
                }
                let heights = self.equal_group_heights(
                    *column,
                    *available_height,
                    metrics,
                )?;
                let ratios = self.group_ratios_from_heights(*column, &heights)?;
                if ratios == current.group_ratios {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.group_ratios = ratios.clone();
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::GroupRatios(ratios)),
                    inverse: Some(DockCommand::RestoreGroupRatios {
                        column: *column,
                        ratios: current.group_ratios,
                    }),
                    persistent: true,
                })
            }
            DockCommand::ResetMainColumnRatios { side } => {
                let Some(area) = self.main_area(*side).copied() else {
                    return Ok(CommandApplication::persistent());
                };
                match self.nodes.get(area.root) {
                    Some(Node::Column(_)) => return Ok(CommandApplication::persistent()),
                    Some(Node::Split(split)) if split.axis == Axis::X => {
                        let equal = equal_ratios(split.children.len());
                        if split.ratios == equal {
                            return Ok(CommandApplication::persistent());
                        }
                        if let Some(Node::Split(split)) = self.nodes.get_mut(area.root) {
                            split.ratios = equal;
                        }
                        Ok(CommandApplication::persistent())
                    }
                    Some(_) => Err(CommandError::InvalidTarget {
                        reason: "main dock root is not a horizontal column split",
                    }),
                    None => Err(CommandError::InvalidTarget {
                        reason: "main dock root is missing",
                    }),
                }
            }
            DockCommand::RestoreGroupRatios { column, ratios } => {
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.group_ratios.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget {
                        reason: "RestoreGroupRatios target must be a Column node",
                    }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                let count = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.groups.len(),
                    _ => 0,
                };
                if ratios.len() != count {
                    return Err(CommandError::InvalidValue {
                        field: "ratios",
                        reason: "ratio count must match group count",
                    });
                }
                if let Some(reason) = ratio_issues(ratios, count).first() {
                    return Err(CommandError::InvalidValue { field: "ratios", reason: *reason });
                }
                if current == *ratios {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.group_ratios = ratios.clone();
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::GroupRatios(ratios.clone())),
                    inverse: Some(DockCommand::RestoreGroupRatios {
                        column: *column,
                        ratios: current,
                    }),
                    persistent: true,
                })
            }
            DockCommand::Float { node, bounds } => {
                // Bounds use model-space logical pixels. Visible-screen clamping
                // belongs to the geometry/render adapter, not to Core/GPUI.
                if !self.nodes.contains_key(*node) {
                    return Err(CommandError::MissingNode { node: *node });
                }
                bounds.validate_non_empty().map_err(|error| CommandError::InvalidValue {
                    field: "bounds",
                    reason: geometry_error_reason(error),
                })?;
                if self.profile == Profile::Photoshop
                    && !matches!(self.nodes.get(*node), Some(Node::Panel(_)) | Some(Node::Tabs(_)))
                {
                    return Err(CommandError::UnsupportedForProfile {
                        profile: self.profile,
                        reason: "only a panel or tab group can become a floating root",
                    });
                }
                let next_z = self.next_floating_z_order()?;
                let panel_root = matches!(self.nodes.get(*node), Some(Node::Panel(_)));
                self.remember_subtree_dock_locations(*node);
                self.detach_node(*node)?;
                let root = if panel_root && self.profile == Profile::Photoshop {
                    self.nodes.insert(Node::Tabs(crate::Tabs {
                        children: vec![*node],
                        active_index: 0,
                        collapsed: false,
                    }))
                } else {
                    *node
                };
                let area_id = self.dock_areas.insert(DockArea::floating(root));
                self.floating_groups.insert(FloatingGroup::new(area_id, *bounds, next_z));
                Ok(CommandApplication::persistent())
            }
            DockCommand::MoveFloating { group, origin } => {
                // Do not clamp to a monitor/work-area here: the adapter applies
                // its visible-area policy; Core stores finite logical coordinates.
                if !origin.is_finite() {
                    return Err(CommandError::InvalidValue { field: "origin", reason: "origin coordinates must be finite" });
                }
                let floating = self.floating_groups.get(*group).copied().ok_or(
                    CommandError::MissingFloatingGroup { group: *group },
                )?;
                let bounds = Rect::new(*origin, floating.bounds.size);
                bounds.validate_non_empty().map_err(|error| CommandError::InvalidValue {
                    field: "origin",
                    reason: geometry_error_reason(error),
                })?;
                if floating.bounds.origin == *origin {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds.origin = *origin;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::MoveFloating { group: *group, origin: floating.bounds.origin }),
                    persistent: true,
                })
            }
            DockCommand::ResizeFloating { group, size } => {
                size.validate_positive().map_err(|error| CommandError::InvalidValue {
                    field: "size",
                    reason: geometry_error_reason(error),
                })?;
                let floating = self.floating_groups.get(*group).copied().ok_or(
                    CommandError::MissingFloatingGroup { group: *group },
                )?;
                let root = self.dock_areas.get(floating.area).ok_or(CommandError::InvalidTarget {
                    reason: "floating group references a missing dock area",
                })?.root;
                let minimum = self.minimum_node_size(root, metrics)?;
                let actual = Size::new(size.width.max(minimum.width), size.height.max(minimum.height));
                let new_bounds = Rect::new(floating.bounds.origin, actual);
                new_bounds.validate_non_empty().map_err(|error| CommandError::InvalidValue {
                    field: "size",
                    reason: geometry_error_reason(error),
                })?;
                if actual == floating.bounds.size {
                    return Ok(CommandApplication {
                        actual_value: Some(CommandValue::FloatingSize(actual)),
                        inverse: None,
                        persistent: true,
                    });
                }
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.bounds = new_bounds;
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::FloatingSize(actual)),
                    inverse: Some(DockCommand::ResizeFloating { group: *group, size: floating.bounds.size }),
                    persistent: true,
                })
            }
            DockCommand::RaiseFloating { group } => {
                let current = self.floating_groups.get(*group).copied().ok_or(
                    CommandError::MissingFloatingGroup { group: *group },
                )?;
                let max_other = self.floating_groups.iter()
                    .filter(|(other_id, _)| *other_id != *group)
                    .map(|(_, value)| value.z_order)
                    .max();
                if max_other.is_none_or(|max| current.z_order > max) {
                    return Ok(CommandApplication::persistent());
                }
                let max_z = self.floating_groups.iter().map(|(_, value)| value.z_order).max().unwrap_or(-1);
                let next_z = max_z.checked_add(1).ok_or(CommandError::InvalidValue {
                    field: "z_order",
                    reason: "floating z-order overflow",
                })?;
                if let Some(value) = self.floating_groups.get_mut(*group) {
                    value.z_order = next_z;
                }
                Ok(CommandApplication::persistent())
            }
            DockCommand::DockFloating { group, target } => {
                let root = self.floating_groups.get(*group).and_then(|floating| {
                    self.dock_areas.get(floating.area).map(|area| area.root)
                }).ok_or(CommandError::MissingFloatingGroup { group: *group })?;
                let panel_nodes = self.panel_nodes_in_subtree(root);
                self.dock_floating_group(*group, *target, metrics)?;
                for panel_node in panel_nodes {
                    self.remember_panel_dock_location(panel_node);
                }
                Ok(CommandApplication::persistent())
            }
            DockCommand::CollapseGroup { group, collapsed } => {
                let old = match self.nodes.get(*group) {
                    Some(Node::Tabs(tabs)) => tabs.collapsed,
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "CollapseGroup target must be a Tabs node" }),
                    None => return Err(CommandError::MissingNode { node: *group }),
                };
                if old == *collapsed {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Tabs(tabs)) = self.nodes.get_mut(*group) {
                    tabs.collapsed = *collapsed;
                }
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::CollapseGroup { group: *group, collapsed: old }),
                    persistent: true,
                })
            }
            DockCommand::SetColumnMode { column, mode } => {
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "SetColumnMode target must be a Column node" }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if current.mode == *mode {
                    return Ok(CommandApplication::persistent());
                }
                let width = match *mode {
                    ColumnMode::IconsOnly => metrics.rail_width,
                    ColumnMode::Expanded if current.mode == ColumnMode::IconsOnly => {
                        metrics.default_column_width.max(self.minimum_column_width(*column, metrics)?)
                    }
                    ColumnMode::Expanded => current.width.max(self.minimum_column_width(*column, metrics)?),
                };
                if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
                    value.mode = *mode;
                    value.width = width;
                }
                self.sync_main_column_ratios(*column)?;
                Ok(CommandApplication::persistent())
            }
            DockCommand::OpenFlyout { column, group } => {
                let groups = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => {
                        if value.mode != ColumnMode::IconsOnly {
                            return Err(CommandError::InvalidTarget { reason: "flyout column must be in IconsOnly mode" });
                        }
                        value.groups.clone()
                    }
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "OpenFlyout target must be a Column node" }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                if !groups.contains(group) {
                    return Err(CommandError::InvalidTarget { reason: "flyout group is not a member of the column" });
                }
                if !matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
                    return Err(CommandError::InvalidTarget { reason: "flyout target must be a Tabs node" });
                }
                if self.flyout == Some(FlyoutState::new(*column, *group)) {
                    return Ok(CommandApplication::transient());
                }
                self.flyout = Some(FlyoutState::new(*column, *group));
                Ok(CommandApplication::transient())
            }
            DockCommand::CloseFlyout => {
                if self.flyout.is_none() {
                    return Ok(CommandApplication::transient());
                }
                self.flyout = None;
                Ok(CommandApplication::transient())
            }
            DockCommand::SetChrome { mode } => {
                let old = self.chrome;
                if old == *mode {
                    return Ok(CommandApplication::persistent());
                }
                self.chrome = *mode;
                Ok(CommandApplication {
                    actual_value: None,
                    inverse: Some(DockCommand::SetChrome { mode: old }),
                    persistent: true,
                })
            }
        }
    }

    fn validate_drop_target(
        &self,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        match target {
            DropTarget::Tabs { group, index } => match self.nodes.get(group) {
                Some(Node::Tabs(tabs)) if index <= tabs.children.len() => Ok(()),
                Some(Node::Tabs(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "tab insertion index is outside the group",
                }),
                Some(_) => Err(CommandError::InvalidTarget { reason: "tab target must be a Tabs node" }),
                None => Err(CommandError::MissingNode { node: group }),
            },
            DropTarget::GroupEdge { group, zone } => {
                match self.nodes.get(group) {
                    Some(Node::Tabs(_)) => {}
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "group-edge target must be a Tabs node" }),
                    None => return Err(CommandError::MissingNode { node: group }),
                }
                if self.profile == Profile::Photoshop {
                    match zone {
                        DropZone::Left | DropZone::Right => {
                            let Some(column) = self.find_owner_column(group) else {
                                return Err(CommandError::InvalidTarget {
                                    reason: if self.find_floating_root_containing(group).is_some() {
                                        "horizontal splits are not supported inside a floating stack"
                                    } else {
                                        "horizontal group splits require a main-dock column"
                                    },
                                });
                            };
                            if self.main_column_position(column).is_none() {
                                return Err(CommandError::InvalidTarget {
                                    reason: "horizontal group splits require a column in a main dock",
                                });
                            }
                        }
                        DropZone::Top | DropZone::Bottom => {
                            if self.find_owner_column(group).is_none()
                                && self.find_floating_root_containing(group).is_none()
                            {
                                return Err(CommandError::InvalidTarget {
                                    reason: "vertical group splits require a column or floating stack",
                                });
                            }
                        }
                    }
                }
                Ok(())
            },
            DropTarget::MainColumn { side, index } => {
                let columns = self.main_columns(side)?;
                if index <= columns.len() {
                    Ok(())
                } else {
                    Err(CommandError::InvalidValue {
                        field: "index",
                        reason: "main column insertion index is outside the dock",
                    })
                }
            }
            DropTarget::RailSlot { column, index } => match self.nodes.get(column) {
                Some(Node::Column(value)) if value.mode != ColumnMode::IconsOnly => {
                    Err(CommandError::InvalidTarget { reason: "rail insertion target must use IconsOnly mode" })
                }
                Some(Node::Column(value)) if index <= value.groups.len() => Ok(()),
                Some(Node::Column(_)) => Err(CommandError::InvalidValue {
                    field: "index",
                    reason: "rail insertion index is outside the column",
                }),
                Some(_) => Err(CommandError::InvalidTarget { reason: "rail target must be a Column node" }),
                None => Err(CommandError::MissingNode { node: column }),
            },
        }.and_then(|()| {
            let _ = metrics;
            Ok(())
        })
    }

    fn insert_node_for_target(
        &mut self,
        node: NodeId,
        target: DropTarget,
        metrics: &DockMetrics,
        edge_ratio: Option<f32>,
    ) -> Result<(), CommandError> {
        self.validate_drop_target(target, metrics)?;
        match target {
            DropTarget::Tabs { group, index } => {
                if !matches!(self.nodes.get(node), Some(Node::Panel(_))) {
                    return Err(CommandError::InvalidTarget { reason: "Tabs target accepts a Panel leaf; move a group to a group or column target" });
                }
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::InvalidTarget { reason: "tab target changed unexpectedly" });
                };
                tabs.children.insert(index, node);
                tabs.active_index = index;
                Ok(())
            }
            DropTarget::GroupEdge { group, zone } => {
                let group_node = self.group_node_for(node)?;
                self.insert_group_edge(group_node, group, zone, edge_ratio, metrics)
            }
            DropTarget::MainColumn { side, index } => {
                let group = self.group_node_for(node)?;
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![group],
                    group_ratios: vec![1.0],
                }));
                self.insert_existing_column_at(side, index, column, None)
            }
            DropTarget::RailSlot { column, index } => {
                let group = self.group_node_for(node)?;
                self.insert_group_at_column(column, group, index, None)
            }
        }
    }

    fn group_node_for(&mut self, node: NodeId) -> Result<NodeId, CommandError> {
        match self.nodes.get(node).cloned() {
            Some(Node::Tabs(_)) => Ok(node),
            Some(Node::Panel(_)) => Ok(self.nodes.insert(Node::Tabs(crate::Tabs {
                children: vec![node],
                active_index: 0,
                collapsed: false,
            }))),
            Some(_) if self.profile == Profile::Generic => Ok(self.nodes.insert(Node::Tabs(crate::Tabs {
                children: vec![node],
                active_index: 0,
                collapsed: false,
            }))),
            Some(_) => Err(CommandError::UnsupportedForProfile {
                profile: self.profile,
                reason: "Photoshop columns and floating stacks can contain Tabs groups or Panel leaves only",
            }),
            None => Err(CommandError::MissingNode { node }),
        }
    }

    fn insert_existing_column_at(
        &mut self,
        side: DockSide,
        index: usize,
        new_column: NodeId,
        split_ratio: Option<(f32, usize)>,
    ) -> Result<(), CommandError> {
        if !matches!(self.nodes.get(new_column), Some(Node::Column(_))) {
            return Err(CommandError::InvalidTarget { reason: "inserted main column is not a Column node" });
        }
        let Some(area_id) = self.main_areas[side_index(side)] else {
            if index != 0 {
                return Err(CommandError::InvalidValue { field: "index", reason: "first column on an empty dock must use index zero" });
            }
            let area_id = self.dock_areas.insert(DockArea::main(side, new_column));
            self.main_areas[side_index(side)] = Some(area_id);
            return Ok(());
        };
        let area_root = self.dock_areas.get(area_id).ok_or(CommandError::InvalidTarget {
            reason: "main dock side references a missing area",
        })?.root;
        match self.nodes.get(area_root).cloned() {
            Some(Node::Column(_)) => {
                if index > 1 {
                    return Err(CommandError::InvalidValue { field: "index", reason: "column insertion index is outside the dock" });
                }
                let children = if index == 0 { vec![new_column, area_root] } else { vec![area_root, new_column] };
                let ratios = match split_ratio {
                    Some((ratio, target_index))
                        if target_index == 0 && ratio.is_finite() && ratio > 0.0 && ratio < 1.0 =>
                    {
                        if index == 0 { vec![ratio, 1.0 - ratio] } else { vec![1.0 - ratio, ratio] }
                    }
                    Some((_, target_index)) if target_index != 0 => {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target does not identify the existing column",
                        });
                    }
                    Some(_) => return Err(CommandError::InvalidValue { field: "ratio", reason: "split ratio must be between zero and one" }),
                    None => equal_ratios(2),
                };
                let split = self.nodes.insert(Node::Split(crate::Split {
                    axis: Axis::X,
                    ratios,
                    children,
                }));
                if let Some(area) = self.dock_areas.get_mut(area_id) {
                    area.root = split;
                }
                Ok(())
            }
            Some(Node::Split(mut split)) if split.axis == Axis::X => {
                if index > split.children.len() {
                    return Err(CommandError::InvalidValue { field: "index", reason: "column insertion index is outside the dock" });
                }
                if split.children.iter().any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_)))) {
                    return Err(CommandError::InvalidTarget { reason: "main horizontal split contains a non-Column child" });
                }
                let old_len = split.children.len();
                let ratios = if let Some((ratio, target_index)) = split_ratio {
                    if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                        return Err(CommandError::InvalidValue { field: "ratio", reason: "split ratio must be between zero and one" });
                    }
                    if target_index >= old_len || index > old_len {
                        return Err(CommandError::InvalidTarget {
                            reason: "split ratio target or column insertion index is outside the dock",
                        });
                    }
                    let mut weights = if ratio_issues(&split.ratios, old_len).is_empty() {
                        split.ratios.clone()
                    } else {
                        equal_ratios(old_len)
                    };
                    let target_weight = weights[target_index];
                    weights[target_index] = target_weight * (1.0 - ratio);
                    weights.insert(index, target_weight * ratio);
                    normalized_ratios(weights.into_iter().map(Some).collect())
                } else {
                    equal_ratios(old_len + 1)
                };
                split.children.insert(index, new_column);
                split.ratios = ratios;
                if let Some(Node::Split(current)) = self.nodes.get_mut(area_root) {
                    *current = split;
                }
                Ok(())
            }
            Some(_) => Err(CommandError::InvalidTarget { reason: "main dock root must be a Column or Split(X) of Columns" }),
            None => Err(CommandError::InvalidTarget { reason: "main dock root is missing" }),
        }
    }

    fn insert_group_at_column(
        &mut self,
        column_id: NodeId,
        group_id: NodeId,
        index: usize,
        split_ratio: Option<(f32, usize)>,
    ) -> Result<(), CommandError> {
        if group_id == column_id {
            return Err(CommandError::InvalidTarget { reason: "a column cannot contain itself" });
        }
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column.clone(),
            Some(_) => return Err(CommandError::InvalidTarget { reason: "group insertion target must be a Column node" }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if index > column.groups.len() {
            return Err(CommandError::InvalidValue { field: "index", reason: "group insertion index is outside the column" });
        }
        if column.groups.contains(&group_id) {
            return Err(CommandError::InvalidTarget { reason: "group is already in the target column" });
        }
        let mut groups = column.groups.clone();
        groups.insert(index, group_id);
        let ratios = if let Some((ratio, target_index)) = split_ratio {
            if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                return Err(CommandError::InvalidValue { field: "ratio", reason: "split ratio must be between zero and one" });
            }
            let old_len = column.groups.len();
            if target_index >= old_len {
                return Err(CommandError::InvalidTarget {
                    reason: "split ratio target does not identify an existing group",
                });
            }
            let mut weights = if column.group_ratios.len() == old_len
                && column.group_ratios.iter().all(|value| value.is_finite() && *value >= 0.0)
                && column.group_ratios.iter().copied().sum::<f32>().is_finite()
                && column.group_ratios.iter().copied().sum::<f32>() > f32::EPSILON
            {
                normalized_ratios(column.group_ratios.iter().copied().map(Some).collect())
            } else {
                equal_ratios(old_len)
            };
            let target_weight = weights[target_index];
            weights[target_index] = target_weight * (1.0 - ratio);
            weights.insert(index, target_weight * ratio);
            normalized_ratios(weights.into_iter().map(Some).collect())
        } else {
            equal_ratios(groups.len())
        };
        let Some(Node::Column(column)) = self.nodes.get_mut(column_id) else {
            return Err(CommandError::InvalidTarget { reason: "target column changed unexpectedly" });
        };
        column.groups = groups;
        column.group_ratios = ratios;
        Ok(())
    }

    fn insert_group_edge(
        &mut self,
        new_group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        ratio: Option<f32>,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        if new_group == target_group {
            return Ok(());
        }
        if !matches!(self.nodes.get(new_group), Some(Node::Tabs(_)))
            || !matches!(self.nodes.get(target_group), Some(Node::Tabs(_)))
        {
            return Err(CommandError::InvalidTarget { reason: "group-edge operations require Tabs nodes" });
        }
        let split_ratio = ratio.unwrap_or(0.5);
        if !split_ratio.is_finite() || split_ratio <= 0.0 || split_ratio >= 1.0 {
            return Err(CommandError::InvalidValue { field: "ratio", reason: "split ratio must be between zero and one" });
        }

        if self.profile == Profile::Photoshop {
            if let Some(column_id) = self.find_owner_column(target_group) {
                match zone {
                    DropZone::Top | DropZone::Bottom => {
                        let groups = match self.nodes.get(column_id) {
                            Some(Node::Column(column)) => column.groups.clone(),
                            _ => return Err(CommandError::InvalidTarget { reason: "target group has no owning column" }),
                        };
                        let target_index = groups.iter().position(|id| *id == target_group).ok_or(
                            CommandError::InvalidTarget { reason: "target group is not in its owning column" },
                        )?;
                        let index = if zone == DropZone::Top { target_index } else { target_index + 1 };
                        self.insert_group_at_column(column_id, new_group, index, Some((split_ratio, target_index)))
                    }
                    DropZone::Left | DropZone::Right => {
                        let (side, column_index) = self.main_column_position(column_id).ok_or(
                            CommandError::InvalidTarget { reason: "horizontal group splits require a column in a main dock" },
                        )?;
                        let insert_at = if zone == DropZone::Left { column_index } else { column_index + 1 };
                        let column = self.nodes.insert(Node::Column(crate::Column {
                            width: metrics.default_column_width,
                            mode: ColumnMode::Expanded,
                            groups: vec![new_group],
                            group_ratios: vec![1.0],
                        }));
                        self.insert_existing_column_at(side, insert_at, column, Some((split_ratio, column_index)))
                    }
                }
            } else {
                let floating_root = self.find_floating_root_containing(target_group).ok_or(
                    CommandError::InvalidTarget { reason: "target group is not in a Photoshop column or floating stack" },
                )?;
                match zone {
                    DropZone::Left | DropZone::Right => Err(CommandError::InvalidTarget {
                        reason: "horizontal splits are not supported inside a floating stack",
                    }),
                    DropZone::Top | DropZone::Bottom => {
                        if floating_root == target_group {
                            let (children, ratios) = if zone == DropZone::Top {
                                (vec![new_group, target_group], vec![split_ratio, 1.0 - split_ratio])
                            } else {
                                (vec![target_group, new_group], vec![1.0 - split_ratio, split_ratio])
                            };
                            let split = self.nodes.insert(Node::Split(crate::Split {
                                axis: Axis::Y,
                                children,
                                ratios,
                            }));
                            self.set_area_root(target_group, split)?;
                            Ok(())
                        } else {
                            let parent = self.find_parent(target_group).ok_or(
                                CommandError::InvalidTarget { reason: "floating group parent was not found" },
                            )?;
                            match self.nodes.get(parent) {
                                Some(Node::Split(split)) if split.axis == Axis::Y && split.children.contains(&target_group) => {
                                    let target_index = split.children.iter().position(|id| *id == target_group).ok_or(
                                        CommandError::InvalidTarget {
                                            reason: "target group is not present in the floating stack",
                                        },
                                    )?;
                                    let insert_at = if zone == DropZone::Top { target_index } else { target_index + 1 };
                                    let old_len = split.children.len();
                                    let mut weights = if split.ratios.len() == old_len
                                        && split.ratios.iter().all(|value| value.is_finite() && *value >= 0.0)
                                        && split.ratios.iter().copied().sum::<f32>().is_finite()
                                        && split.ratios.iter().copied().sum::<f32>() > f32::EPSILON
                                    {
                                        normalized_ratios(split.ratios.iter().copied().map(Some).collect())
                                    } else {
                                        equal_ratios(old_len)
                                    };
                                    let target_weight = weights[target_index];
                                    weights[target_index] = target_weight * (1.0 - split_ratio);
                                    weights.insert(insert_at, target_weight * split_ratio);
                                    let new_ratios = normalized_ratios(weights.into_iter().map(Some).collect());
                                    let Some(Node::Split(split)) = self.nodes.get_mut(parent) else {
                                        return Err(CommandError::InvalidTarget { reason: "floating stack changed unexpectedly" });
                                    };
                                    split.children.insert(insert_at, new_group);
                                    split.ratios = new_ratios;
                                    Ok(())
                                }
                                _ => Err(CommandError::InvalidTarget { reason: "floating stack must be a vertical Split of Tabs" }),
                            }
                        }
                    }
                }
            }
        } else {
            let parent = self.find_parent(target_group);
            let axis = match zone {
                DropZone::Left | DropZone::Right => Axis::X,
                DropZone::Top | DropZone::Bottom => Axis::Y,
            };
            let children = match zone {
                DropZone::Left | DropZone::Top => vec![new_group, target_group],
                DropZone::Right | DropZone::Bottom => vec![target_group, new_group],
            };
            let ratios = match zone {
                DropZone::Left | DropZone::Top => vec![split_ratio, 1.0 - split_ratio],
                DropZone::Right | DropZone::Bottom => vec![1.0 - split_ratio, split_ratio],
            };
            let split = self.nodes.insert(Node::Split(crate::Split { axis, children, ratios }));
            if let Some(parent) = parent {
                self.replace_child(parent, target_group, split)?;
            } else {
                self.set_area_root(target_group, split)?;
            }
            Ok(())
        }
    }

    fn dock_floating_group(
        &mut self,
        group_id: FloatingGroupId,
        target: DropTarget,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let floating = self.floating_groups.get(group_id).copied().ok_or(
            CommandError::MissingFloatingGroup { group: group_id },
        )?;
        let root = self.dock_areas.get(floating.area).ok_or(CommandError::InvalidTarget {
            reason: "floating group references a missing dock area",
        })?.root;
        self.validate_drop_target(target, metrics)?;
        if let DropTarget::Tabs { group: target_group, .. } = target {
            if self.is_descendant_or_same(root, target_group) {
                return Ok(());
            }
            if matches!(self.nodes.get(root), Some(Node::Split(split)) if split.axis == Axis::Y) {
                return Err(CommandError::InvalidTarget {
                    reason: "a floating stack cannot be merged into a tab strip; target a dock column or rail slot",
                });
            }
        }
        match (self.nodes.get(root).cloned(), target) {
            (Some(Node::Split(split)), DropTarget::MainColumn { side, index }) if split.axis == Axis::Y => {
                if split.children.iter().any(|id| !matches!(self.nodes.get(*id), Some(Node::Tabs(_)))) {
                    return Err(CommandError::InvalidTarget { reason: "floating stack contains a non-Tabs group" });
                }
                self.detach_node(root)?;
                self.nodes.remove(root);
                let column = self.nodes.insert(Node::Column(crate::Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: split.children,
                    group_ratios: normalized_ratios(split.ratios.into_iter().map(Some).collect()),
                }));
                self.insert_existing_column_at(side, index, column, None)
            }
            (Some(Node::Split(split)), DropTarget::RailSlot { column, index }) if split.axis == Axis::Y => {
                if split.children.iter().any(|id| !matches!(self.nodes.get(*id), Some(Node::Tabs(_)))) {
                    return Err(CommandError::InvalidTarget { reason: "floating stack contains a non-Tabs group" });
                }
                self.detach_node(root)?;
                self.nodes.remove(root);
                let mut insert_at = index;
                for child in split.children {
                    self.insert_group_at_column(column, child, insert_at, None)?;
                    insert_at += 1;
                }
                Ok(())
            }
            (Some(Node::Tabs(mut tabs)), DropTarget::Tabs { group: target_group, index }) => {
                self.detach_node(root)?;
                if let Some(Node::Tabs(old_root)) = self.nodes.get_mut(root) {
                    old_root.children.clear();
                    old_root.active_index = 0;
                }
                self.nodes.remove(root);
                if let Some(Node::Tabs(target_tabs)) = self.nodes.get_mut(target_group) {
                    let insert_at = index.min(target_tabs.children.len());
                    let source_active_index = tabs.active_index;
                    let moving_count = tabs.children.len();
                    for (offset, child) in tabs.children.drain(..).enumerate() {
                        target_tabs.children.insert(insert_at + offset, child);
                    }
                    if moving_count > 0 {
                        target_tabs.active_index = insert_at + source_active_index.min(moving_count - 1);
                    }
                } else {
                    return Err(CommandError::InvalidTarget { reason: "tab target changed unexpectedly" });
                }
                Ok(())
            }
            (Some(Node::Split(_)), _) => {
                Err(CommandError::InvalidTarget {
                    reason: "floating split stacks can only be docked into a main column or icon rail",
                })
            }
            (Some(_), _) => {
                self.detach_node(root)?;
                self.insert_node_for_target(root, target, metrics, Some(0.5))
            }
            (None, _) => Err(CommandError::MissingNode { node: root }),
        }
    }

    fn main_columns(&self, side: DockSide) -> Result<Vec<NodeId>, CommandError> {
        let Some(area) = self.main_area(side) else {
            return Ok(Vec::new());
        };
        match self.nodes.get(area.root) {
            Some(Node::Column(_)) => Ok(vec![area.root]),
            Some(Node::Split(split)) if split.axis == Axis::X => {
                if split.children.iter().any(|id| !matches!(self.nodes.get(*id), Some(Node::Column(_)))) {
                    return Err(CommandError::InvalidTarget { reason: "main horizontal split must contain only Column nodes" });
                }
                Ok(split.children.clone())
            }
            Some(_) => Err(CommandError::InvalidTarget { reason: "main dock root must be a Column or Split(X) of Columns" }),
            None => Err(CommandError::InvalidTarget { reason: "main dock root is missing" }),
        }
    }

    fn resolve_open_target(
        &self,
        key: &PanelKey,
        metrics: &DockMetrics,
    ) -> Result<DropTarget, CommandError> {
        if let Some(location) = self.last_dock_locations.get(key).copied() {
            if let Some(target) = self.target_for_dock_location(location) {
                if self.validate_drop_target(target, metrics).is_ok() {
                    return Ok(target);
                }
            }
        }
        self.default_open_target()
    }

    fn target_for_dock_location(&self, location: LastDockLocation) -> Option<DropTarget> {
        if let Ok(columns) = self.main_columns(location.side()) {
            if let Some(column_id) = columns.get(location.column_index()).copied() {
                if let Some(Node::Column(column)) = self.nodes.get(column_id) {
                    if let Some(group) = column.groups.get(location.group_index()).copied() {
                        if let Some(group) = self.first_tabs_in_subtree(group) {
                            let child_count = match self.nodes.get(group) {
                                Some(Node::Tabs(tabs)) => tabs.children.len(),
                                _ => return None,
                            };
                            return Some(DropTarget::Tabs { group, index: child_count });
                        }
                    }

                    if !column.groups.is_empty() {
                        if column.mode == ColumnMode::IconsOnly {
                            return Some(DropTarget::RailSlot {
                                column: column_id,
                                index: location.group_index().min(column.groups.len()),
                            });
                        }
                        if let Some(last_group) = column.groups.last().copied() {
                            return Some(DropTarget::GroupEdge {
                                group: last_group,
                                zone: DropZone::Bottom,
                            });
                        }
                    }
                }
            }
        }

        // Generic-profile trees need not have Column nodes. In that case the
        // closest stable preference is the first tab group under the same side.
        let area = self.main_area(location.side())?;
        let group = self.first_tabs_in_subtree(area.root)?;
        let index = match self.nodes.get(group) {
            Some(Node::Tabs(tabs)) => tabs.children.len(),
            _ => return None,
        };
        Some(DropTarget::Tabs { group, index })
    }

    fn default_open_target(&self) -> Result<DropTarget, CommandError> {
        // Both built-in profiles default to the right dock. Panel-kind-specific
        // placement remains app/descriptor policy and can be supplied explicitly.
        let side = match self.profile {
            Profile::Generic | Profile::Photoshop => DockSide::Right,
        };
        if let Some(area) = self.main_area(side) {
            if let Some(group) = self.first_tabs_in_subtree(area.root) {
                let index = match self.nodes.get(group) {
                    Some(Node::Tabs(tabs)) => tabs.children.len(),
                    _ => return Err(CommandError::InvalidTarget {
                        reason: "profile default placement did not resolve to a Tabs node",
                    }),
                };
                return Ok(DropTarget::Tabs { group, index });
            }
            return Err(CommandError::InvalidTarget {
                reason: "profile default placement has no compatible tab group on the right dock",
            });
        }
        Ok(DropTarget::MainColumn { side, index: 0 })
    }

    fn first_tabs_in_subtree(&self, root: NodeId) -> Option<NodeId> {
        match self.nodes.get(root)? {
            Node::Tabs(_) => Some(root),
            node => node_children(node)
                .into_iter()
                .find_map(|child| self.first_tabs_in_subtree(child)),
        }
    }

    fn panel_nodes_in_subtree(&self, root: NodeId) -> Vec<NodeId> {
        let mut panels = Vec::new();
        self.collect_panel_nodes(root, &mut panels);
        panels
    }

    fn collect_panel_nodes(&self, root: NodeId, panels: &mut Vec<NodeId>) {
        let Some(node) = self.nodes.get(root) else {
            return;
        };
        if matches!(node, Node::Panel(_)) {
            panels.push(root);
            return;
        }
        for child in node_children(node) {
            self.collect_panel_nodes(child, panels);
        }
    }

    fn remember_subtree_dock_locations(&mut self, root: NodeId) {
        for panel_node in self.panel_nodes_in_subtree(root) {
            self.remember_panel_dock_location(panel_node);
        }
    }

    fn remember_panel_dock_location(&mut self, panel_node: NodeId) {
        let Some(Node::Panel(panel)) = self.nodes.get(panel_node) else {
            return;
        };
        let Some(instance) = self.panel_instances.get(panel.panel_id) else {
            return;
        };
        let key = instance.key.clone();
        if let Some(location) = self.dock_location_for_node(panel_node) {
            self.last_dock_locations.insert(key, location);
        }
    }

    fn dock_location_for_node(&self, node_id: NodeId) -> Option<LastDockLocation> {
        for side in [DockSide::Left, DockSide::Right] {
            let Some(area) = self.main_area(side) else {
                continue;
            };
            if !self.is_descendant_or_same(area.root, node_id) {
                continue;
            }
            if let Ok(columns) = self.main_columns(side) {
                for (column_index, column_id) in columns.iter().copied().enumerate() {
                    let Some(Node::Column(column)) = self.nodes.get(column_id) else {
                        continue;
                    };
                    if let Some(group_index) = column.groups.iter().position(|group| {
                        self.is_descendant_or_same(*group, node_id)
                    }) {
                        return Some(LastDockLocation::new(side, column_index, group_index));
                    }
                }
            }
            // The Generic profile may use a tree without Column nodes. It still
            // remembers the side; indices zero are the deterministic fallback.
            return Some(LastDockLocation::new(side, 0, 0));
        }
        None
    }

    fn find_panel_node(&self, panel_id: PanelId) -> Option<NodeId> {
        self.nodes.iter().find_map(|(id, node)| match node {
            Node::Panel(panel) if panel.panel_id == panel_id => Some(id),
            _ => None,
        })
    }

    fn tab_parent_index(&self, node_id: NodeId) -> Option<(NodeId, usize)> {
        let parent = self.find_parent(node_id)?;
        match self.nodes.get(parent) {
            Some(Node::Tabs(tabs)) => tabs.children.iter().position(|id| *id == node_id).map(|index| (parent, index)),
            _ => None,
        }
    }

    fn find_parent(&self, child: NodeId) -> Option<NodeId> {
        self.nodes.iter().find_map(|(parent, node)| {
            node_children(node).contains(&child).then_some(parent)
        })
    }

    fn find_owner_column(&self, group: NodeId) -> Option<NodeId> {
        self.nodes.iter().find_map(|(id, node)| match node {
            Node::Column(column) if column.groups.contains(&group) => Some(id),
            _ => None,
        })
    }

    fn main_column_position(&self, column_id: NodeId) -> Option<(DockSide, usize)> {
        for side in [DockSide::Left, DockSide::Right] {
            if let Ok(columns) = self.main_columns(side) {
                if let Some(index) = columns.iter().position(|id| *id == column_id) {
                    return Some((side, index));
                }
            }
        }
        None
    }

    fn find_floating_root_containing(&self, node_id: NodeId) -> Option<NodeId> {
        self.floating_groups.values().find_map(|group| {
            let root = self.dock_areas.get(group.area)?.root;
            self.is_descendant_or_same(root, node_id).then_some(root)
        })
    }

    fn is_descendant_or_same(&self, root: NodeId, target: NodeId) -> bool {
        if root == target {
            return true;
        }
        let Some(node) = self.nodes.get(root) else {
            return false;
        };
        node_children(node).into_iter().any(|child| self.is_descendant_or_same(child, target))
    }

    fn replace_child(
        &mut self,
        parent: NodeId,
        old_child: NodeId,
        new_child: NodeId,
    ) -> Result<(), CommandError> {
        let Some(parent_node) = self.nodes.get_mut(parent) else {
            return Err(CommandError::MissingNode { node: parent });
        };
        let replaced = match parent_node {
            Node::Split(split) => split.children.iter_mut().find(|id| **id == old_child).map(|id| *id = new_child).is_some(),
            Node::Tabs(tabs) => tabs.children.iter_mut().find(|id| **id == old_child).map(|id| *id = new_child).is_some(),
            Node::Column(column) => column.groups.iter_mut().find(|id| **id == old_child).map(|id| *id = new_child).is_some(),
            Node::Panel(_) => false,
        };
        if replaced {
            Ok(())
        } else {
            Err(CommandError::InvalidTarget { reason: "target node is not a child of the expected parent" })
        }
    }

    fn set_area_root(&mut self, old_root: NodeId, new_root: NodeId) -> Result<(), CommandError> {
        if let Some(area) = self.dock_areas.values_mut().find(|area| area.root == old_root) {
            area.root = new_root;
            return Ok(());
        }
        Err(CommandError::InvalidTarget { reason: "target node is not a registered dock-area root" })
    }

    fn detach_node(&mut self, node_id: NodeId) -> Result<(), CommandError> {
        if !self.nodes.contains_key(node_id) {
            return Err(CommandError::MissingNode { node: node_id });
        }
        if let Some(parent) = self.find_parent(node_id) {
            let Some(parent_node) = self.nodes.get_mut(parent) else {
                return Err(CommandError::MissingNode { node: parent });
            };
            match parent_node {
                Node::Split(split) => {
                    if let Some(index) = split.children.iter().position(|id| *id == node_id) {
                        split.children.remove(index);
                        if index < split.ratios.len() {
                            split.ratios.remove(index);
                        }
                    }
                }
                Node::Tabs(tabs) => {
                    let active_node = tabs.children.get(tabs.active_index).copied();
                    if let Some(index) = tabs.children.iter().position(|id| *id == node_id) {
                        tabs.children.remove(index);
                        if index < tabs.children.len() {
                            if active_node == Some(node_id) {
                                tabs.active_index = index;
                            } else if let Some(active_node) = active_node {
                                tabs.active_index = tabs.children.iter().position(|id| *id == active_node).unwrap_or(index);
                            } else {
                                tabs.active_index = index.min(tabs.children.len().saturating_sub(1));
                            }
                        } else {
                            tabs.active_index = tabs.children.len().saturating_sub(1);
                        }
                    }
                }
                Node::Column(column) => {
                    if let Some(index) = column.groups.iter().position(|id| *id == node_id) {
                        column.groups.remove(index);
                        if index < column.group_ratios.len() {
                            column.group_ratios.remove(index);
                        }
                    }
                }
                Node::Panel(_) => return Err(CommandError::InvalidTarget { reason: "Panel leaves cannot own child nodes" }),
            }
            return Ok(());
        }

        for index in 0..self.main_areas.len() {
            if let Some(area_id) = self.main_areas[index] {
                if self.dock_areas.get(area_id).is_some_and(|area| area.root == node_id) {
                    self.main_areas[index] = None;
                    self.dock_areas.remove(area_id);
                    return Ok(());
                }
            }
        }
        let floating = self.floating_groups.iter().find_map(|(id, group)| {
            self.dock_areas.get(group.area).is_some_and(|area| area.root == node_id).then_some((id, group.area))
        });
        if let Some((group_id, area_id)) = floating {
            self.floating_groups.remove(group_id);
            self.dock_areas.remove(area_id);
            return Ok(());
        }
        Err(CommandError::InvalidTarget { reason: "node is not attached to a registered dock area or parent" })
    }

    fn remove_subtree(&mut self, node_id: NodeId) {
        let Some(node) = self.nodes.get(node_id).cloned() else {
            return;
        };
        for child in node_children(&node) {
            self.remove_subtree(child);
        }
        if let Node::Panel(panel) = node {
            self.panel_instances.remove(panel.panel_id);
        }
        self.nodes.remove(node_id);
    }

    fn sync_main_column_ratios(&mut self, column_id: NodeId) -> Result<(), CommandError> {
        let Some((side, _)) = self.main_column_position(column_id) else {
            return Ok(());
        };
        let columns = self.main_columns(side)?;
        if columns.len() < 2 {
            return Ok(());
        }
        let widths: Vec<f32> = columns.iter().map(|id| match self.nodes.get(*id) {
            Some(Node::Column(column)) => Ok(column.width),
            _ => Err(CommandError::InvalidTarget { reason: "main horizontal split contains a non-Column child" }),
        }).collect::<Result<_, _>>()?;
        let total: f32 = widths.iter().copied().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "total dock column width overflowed",
            });
        }
        let ratios: Vec<f32> = widths.iter().map(|width| *width / total).collect();
        let area = self.main_area(side).copied().ok_or(CommandError::InvalidTarget {
            reason: "main dock area disappeared while synchronizing column ratios",
        })?;
        if let Some(Node::Split(split)) = self.nodes.get_mut(area.root) {
            if split.axis != Axis::X {
                return Err(CommandError::InvalidTarget {
                    reason: "main column split must use horizontal axis X",
                });
            }
            split.ratios = ratios;
        }
        Ok(())
    }

    fn group_is_collapsed(&self, group_id: NodeId) -> bool {
        matches!(self.nodes.get(group_id), Some(Node::Tabs(tabs)) if tabs.collapsed)
    }

    fn group_minimum_heights(
        &self,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Vec<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => return Err(CommandError::InvalidTarget {
                reason: "group height constraints require a Column node",
            }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        column.groups.iter().map(|group| {
            if self.group_is_collapsed(*group) {
                Ok(0.0)
            } else {
                Ok(metrics.min_group_height.max(self.minimum_node_size(*group, metrics)?.height))
            }
        }).collect()
    }

    fn group_heights_for_available_height(
        &self,
        column_id: NodeId,
        available_height: f32,
        metrics: &DockMetrics,
    ) -> Result<Vec<f32>, CommandError> {
        let ratios = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            Some(_) => return Err(CommandError::InvalidTarget {
                reason: "group height calculation requires a Column node",
            }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        self.group_heights_for_ratios(column_id, &ratios, available_height, metrics)
    }

    fn group_heights_for_ratios(
        &self,
        column_id: NodeId,
        ratios: &[f32],
        available_height: f32,
        metrics: &DockMetrics,
    ) -> Result<Vec<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => return Err(CommandError::InvalidTarget {
                reason: "group height calculation requires a Column node",
            }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if ratios.len() != column.groups.len() {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason: "ratio count must match group count",
            });
        }
        let mut heights = vec![0.0; column.groups.len()];
        let mut expanded_indices = Vec::new();
        let mut weights = Vec::new();
        let mut fixed_height = 0.0_f32;
        for (index, group) in column.groups.iter().copied().enumerate() {
            if self.group_is_collapsed(group) {
                heights[index] = metrics.collapsed_group_height;
                fixed_height += metrics.collapsed_group_height;
            } else {
                expanded_indices.push(index);
                weights.push(ratios[index]);
            }
        }
        let free_height = available_height - fixed_height;
        if !free_height.is_finite() || free_height < 0.0 {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available height cannot fit the fixed collapsed groups",
            });
        }
        if expanded_indices.is_empty() {
            return Ok(heights);
        }
        if free_height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available height leaves no space for expanded groups",
            });
        }
        let total_weight: f32 = weights.iter().copied().sum();
        if !total_weight.is_finite() || total_weight < 0.0 {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason: "expanded group weights must have a finite sum",
            });
        }
        for (index, weight) in expanded_indices.into_iter().zip(weights) {
            heights[index] = if total_weight <= f32::EPSILON {
                free_height / column.groups.iter().filter(|group| !self.group_is_collapsed(**group)).count() as f32
            } else {
                free_height * weight / total_weight
            };
        }
        Ok(heights)
    }

    fn equal_group_heights(
        &self,
        column_id: NodeId,
        available_height: f32,
        metrics: &DockMetrics,
    ) -> Result<Vec<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => return Err(CommandError::InvalidTarget {
                reason: "group height reset requires a Column node",
            }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        let mut heights = vec![0.0; column.groups.len()];
        let mut expanded_indices = Vec::new();
        let mut fixed_height = 0.0_f32;
        for (index, group) in column.groups.iter().copied().enumerate() {
            if self.group_is_collapsed(group) {
                heights[index] = metrics.collapsed_group_height;
                fixed_height += metrics.collapsed_group_height;
            } else {
                expanded_indices.push(index);
            }
        }
        if expanded_indices.is_empty() {
            return Ok(heights);
        }
        let free_height = available_height - fixed_height;
        let all_minimums = self.group_minimum_heights(column_id, metrics)?;
        let minimums: Vec<f32> = expanded_indices.iter().map(|index| all_minimums[*index]).collect();
        let equal_weights = vec![1.0; expanded_indices.len()];
        let expanded_heights = distribute_heights_with_minimums(free_height, &minimums, &equal_weights)?;
        for (index, height) in expanded_indices.into_iter().zip(expanded_heights) {
            heights[index] = height;
        }
        Ok(heights)
    }

    fn group_ratios_from_heights(
        &self,
        column_id: NodeId,
        heights: &[f32],
    ) -> Result<Vec<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => return Err(CommandError::InvalidTarget {
                reason: "group ratio calculation requires a Column node",
            }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if heights.len() != column.groups.len() {
            return Err(CommandError::InvalidValue {
                field: "heights",
                reason: "group height count must match group count",
            });
        }
        let expanded_height: f32 = column.groups.iter().enumerate()
            .filter(|(_, group)| !self.group_is_collapsed(**group))
            .map(|(index, _)| heights[index])
            .sum();
        let mut ratios = vec![0.0; column.groups.len()];
        if expanded_height <= 0.0 {
            return Ok(column.group_ratios.clone());
        }
        for (index, group) in column.groups.iter().copied().enumerate() {
            if !self.group_is_collapsed(group) {
                ratios[index] = heights[index] / expanded_height;
            }
        }
        Ok(normalized_ratios(ratios.into_iter().map(Some).collect()))
    }

    fn adjust_canvas_for_column_resize(
        &mut self,
        column_id: NodeId,
        width_delta: f32,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let Some((side, _)) = self.main_column_position(column_id) else {
            return Ok(());
        };
        let Some(bounds) = self.canvas.last_known_bounds else {
            // The UI geometry pass will compute the canvas remainder once it
            // has a measured parent viewport.
            return Ok(());
        };
        let new_width = bounds.size.width - width_delta;
        let new_origin = Point::new(
            if side == DockSide::Left {
                bounds.origin.x + width_delta
            } else {
                bounds.origin.x
            },
            bounds.origin.y,
        );
        if !new_width.is_finite() || new_width + PIXEL_TOLERANCE < metrics.min_canvas_width {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "column resize would leave the canvas below min_canvas_width",
            });
        }
        let new_bounds = Rect::new(new_origin, Size::new(new_width, bounds.size.height));
        new_bounds.validate_non_empty().map_err(|error| CommandError::InvalidValue {
            field: "canvas_bounds",
            reason: geometry_error_reason(error),
        })?;
        self.canvas.last_known_bounds = Some(new_bounds);
        Ok(())
    }

    fn minimum_column_width(
        &self,
        column_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => return Err(CommandError::InvalidTarget { reason: "target must be a Column node" }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if column.mode == ColumnMode::IconsOnly {
            return Ok(metrics.rail_width);
        }
        let mut minimum = metrics.min_column_width;
        for group in &column.groups {
            minimum = minimum.max(self.minimum_node_size(*group, metrics)?.width);
        }
        if minimum.is_finite() {
            Ok(minimum)
        } else {
            Err(CommandError::InvalidValue { field: "width", reason: "minimum column width overflowed" })
        }
    }

    fn maximum_column_width(
        &self,
        column_id: NodeId,
    ) -> Result<Option<f32>, CommandError> {
        let column = match self.nodes.get(column_id) {
            Some(Node::Column(column)) => column,
            Some(_) => return Err(CommandError::InvalidTarget {
                reason: "column max-width calculation requires a Column node",
            }),
            None => return Err(CommandError::MissingNode { node: column_id }),
        };
        if column.mode == ColumnMode::IconsOnly {
            return Ok(Some(column.width));
        }

        let mut maximum: Option<f32> = None;
        for group in &column.groups {
            for panel_node in self.panel_nodes_in_subtree(*group) {
                let Some(Node::Panel(panel)) = self.nodes.get(panel_node) else {
                    continue;
                };
                let instance = self.panel_instances.get(panel.panel_id).ok_or(
                    CommandError::MissingPanel { panel: panel.panel_id },
                )?;
                if let Some(max_size) = instance.constraints.max_size {
                    maximum = Some(maximum.map_or(max_size.width, |current| current.min(max_size.width)));
                }
            }
        }
        Ok(maximum)
    }

    fn minimum_node_size(
        &self,
        node_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<Size, CommandError> {
        let node = self.nodes.get(node_id).ok_or(CommandError::MissingNode { node: node_id })?;
        let size = match node {
            Node::Panel(panel) => self.panel_instances.get(panel.panel_id).ok_or(
                CommandError::MissingPanel { panel: panel.panel_id },
            )?.constraints.min_size,
            Node::Tabs(tabs) => {
                let mut width: f32 = 0.0;
                let mut height: f32 = 0.0;
                for child in &tabs.children {
                    let child_size = self.minimum_node_size(*child, metrics)?;
                    width = width.max(child_size.width);
                    height = height.max(child_size.height);
                }
                Size::new(width, height)
            }
            Node::Split(split) => {
                let mut width: f32 = 0.0;
                let mut height: f32 = 0.0;
                for child in &split.children {
                    let child_size = self.minimum_node_size(*child, metrics)?;
                    match split.axis {
                        Axis::X => {
                            width += child_size.width;
                            height = height.max(child_size.height);
                        }
                        Axis::Y => {
                            width = width.max(child_size.width);
                            height += child_size.height;
                        }
                    }
                }
                Size::new(width, height)
            }
            Node::Column(column) => {
                let mut width = if column.mode == ColumnMode::IconsOnly { metrics.rail_width } else { metrics.min_column_width };
                let mut height = 0.0;
                for group in &column.groups {
                    let child_size = self.minimum_node_size(*group, metrics)?;
                    width = width.max(child_size.width);
                    height += child_size.height;
                }
                Size::new(width, height)
            }
        };
        if size.is_finite() && size.width > 0.0 && size.height > 0.0 {
            Ok(size)
        } else {
            Err(CommandError::InvalidValue { field: "size", reason: "computed minimum size is not finite and positive" })
        }
    }

    fn next_floating_z_order(&self) -> Result<i32, CommandError> {
        match self.floating_groups.values().map(|group| group.z_order).max() {
            Some(value) => value.checked_add(1).ok_or(CommandError::InvalidValue {
                field: "z_order",
                reason: "floating z-order overflow",
            }),
            None => Ok(0),
        }
    }

    fn same_state(&self, other: &Self) -> bool {
        self.nodes.iter().eq(other.nodes.iter())
            && self.panel_instances.iter().eq(other.panel_instances.iter())
            && self.dock_areas.iter().eq(other.dock_areas.iter())
            && self.main_areas == other.main_areas
            && self.floating_groups.iter().eq(other.floating_groups.iter())
            && self.last_dock_locations == other.last_dock_locations
            && self.canvas == other.canvas
            && self.profile == other.profile
            && self.version == other.version
            && self.chrome == other.chrome
            && self.flyout == other.flyout
    }

    fn changed_node_ids(&self, other: &Self) -> Vec<NodeId> {
        let mut changed = Vec::new();
        for (id, node) in self.nodes.iter() {
            if other.nodes.get(id) != Some(node) {
                changed.push(id);
            }
        }
        for (id, _) in other.nodes.iter() {
            if !self.nodes.contains_key(id) {
                changed.push(id);
            }
        }
        changed
    }


    /// Validates the entire model and returns every detected issue.
    ///
    /// Metrics are supplied explicitly so callers and tests use the same
    /// thresholds as the core algorithms. An empty layout is valid: the canvas
    /// always exists, while empty main dock areas are represented by None.
    pub fn validate(&self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let metrics_result = metrics.validate();
        if let Err(reason) = metrics_result {
            errors.push(ValidationError::InvalidMetrics { reason });
        }
        let metrics_are_valid = metrics_result.is_ok();

        if let Err(reason) = self.canvas.validate() {
            errors.push(ValidationError::InvalidCanvas {
                reason: geometry_error_reason(reason),
            });
        }
        if metrics_are_valid {
            if let Some(bounds) = self.canvas.last_known_bounds()
                && bounds.size.width + PIXEL_TOLERANCE < metrics.min_canvas_width
            {
                errors.push(ValidationError::InvalidCanvas {
                    reason: "canvas width is below min_canvas_width",
                });
            }
        }

        let mut panel_placements: HashMap<PanelId, NodeId> = HashMap::new();

        for (node_id, node) in self.nodes.iter() {
            match node {
                Node::Split(split) => {
                    if split.children.len() < 2 {
                        errors.push(ValidationError::InvalidSplit {
                            node: node_id,
                            reason: "must contain at least two children",
                        });
                    }
                    for reason in ratio_issues(&split.ratios, split.children.len()) {
                        errors.push(ValidationError::InvalidSplit {
                            node: node_id,
                            reason,
                        });
                    }
                }
                Node::Tabs(tabs) => {
                    if tabs.children.is_empty() {
                        errors.push(ValidationError::InvalidTabs {
                            node: node_id,
                            reason: "must contain at least one child",
                        });
                    }
                    if tabs.active_index >= tabs.children.len() {
                        errors.push(ValidationError::InvalidTabs {
                            node: node_id,
                            reason: "active_index is outside children",
                        });
                    }
                    if self.profile == Profile::Photoshop {
                        for child in &tabs.children {
                            if matches!(self.nodes.get(*child), Some(Node::Panel(_))) {
                                continue;
                            }
                            if self.nodes.contains_key(*child) {
                                errors.push(ValidationError::InvalidTabs {
                                    node: node_id,
                                    reason: "Photoshop Tabs children must be Panel nodes",
                                });
                            }
                        }
                    }
                }
                Node::Panel(panel) => {
                    if !self.panel_instances.contains_key(panel.panel_id) {
                        errors.push(ValidationError::MissingPanelInstance {
                            node: node_id,
                            panel: panel.panel_id,
                        });
                    }
                    if let Some(first_node) = panel_placements.get(&panel.panel_id) {
                        errors.push(ValidationError::DuplicatePanelPlacement {
                            panel: panel.panel_id,
                            first_node: *first_node,
                            second_node: node_id,
                        });
                    } else {
                        panel_placements.insert(panel.panel_id, node_id);
                    }
                }
                Node::Column(column) => {
                    if !column.width.is_finite() {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "width must be finite",
                        });
                    } else if column.width <= 0.0 {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "width must be positive",
                        });
                    } else if metrics_are_valid {
                        match column.mode {
                            ColumnMode::Expanded
                                if column.width + PIXEL_TOLERANCE
                                    < metrics.min_column_width =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "expanded width is below min_column_width",
                                });
                            }
                            ColumnMode::IconsOnly
                                if (column.width - metrics.rail_width).abs()
                                    > PIXEL_TOLERANCE =>
                            {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "IconsOnly width must equal rail_width",
                                });
                            }
                            _ => {}
                        }
                    }

                    if column.groups.is_empty() {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason: "must contain at least one group before normalization",
                        });
                    }
                    for reason in ratio_issues(&column.group_ratios, column.groups.len()) {
                        errors.push(ValidationError::InvalidColumn {
                            node: node_id,
                            reason,
                        });
                    }
                    if self.profile == Profile::Photoshop {
                        for group in &column.groups {
                            if matches!(self.nodes.get(*group), Some(Node::Tabs(_))) {
                                continue;
                            }
                            if self.nodes.contains_key(*group) {
                                errors.push(ValidationError::InvalidColumn {
                                    node: node_id,
                                    reason: "Photoshop Column groups must be Tabs nodes",
                                });
                            }
                        }
                    }
                }
            }
        }

        let mut singleton_by_key: HashMap<crate::PanelKey, (PanelId, bool)> = HashMap::new();
        for (panel_id, instance) in self.panel_instances.iter() {
            if let Err(reason) = instance.validate() {
                errors.push(ValidationError::InvalidPanelInstance {
                    panel: panel_id,
                    reason,
                });
            }
            if !panel_placements.contains_key(&panel_id) {
                errors.push(ValidationError::UnplacedPanelInstance { panel: panel_id });
            }

            let key = instance.key.clone();
            let singleton = instance.constraints.singleton;
            if let Some((first_id, first_singleton)) = singleton_by_key.get_mut(&key) {
                if singleton || *first_singleton {
                    errors.push(ValidationError::DuplicateSingletonPanel {
                        key,
                        first: *first_id,
                        second: panel_id,
                    });
                }
                *first_singleton |= singleton;
            } else {
                singleton_by_key.insert(key, (panel_id, singleton));
            }
        }

        // Validate area records and discover the only legal entry points into
        // the tree: registered main sides and floating groups with a floating area.
        let mut registered_areas = HashSet::new();
        let mut roots = Vec::new();
        let mut photoshop_main_columns = 0usize;

        for (area_id, area) in self.dock_areas.iter() {
            if let Err(reason) = area.validate_metadata() {
                errors.push(ValidationError::InvalidDockArea {
                    area: area_id,
                    reason,
                });
            }
        }

        for (side, index) in [(DockSide::Left, 0usize), (DockSide::Right, 1usize)] {
            let Some(area_id) = self.main_areas[index] else {
                continue;
            };
            if !registered_areas.insert(area_id) {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "the same DockAreaId is assigned to more than one main side",
                });
            }

            let Some(area) = self.dock_areas.get(area_id) else {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "references a missing DockAreaId",
                });
                continue;
            };

            if area.kind != DockAreaKind::Main {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "main side must reference a Main DockArea",
                });
            }
            if area.side != Some(side) {
                errors.push(ValidationError::InvalidMainArea {
                    area: area_id,
                    side,
                    reason: "DockArea side does not match its main-area slot",
                });
            }

            if self.nodes.contains_key(area.root) {
                roots.push(area.root);
                if self.profile == Profile::Photoshop
                    && area.kind == DockAreaKind::Main
                    && area.side == Some(side)
                {
                    photoshop_main_columns +=
                        validate_photoshop_main_root(&self.nodes, area_id, area.root, &mut errors);
                }
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: None,
                    missing: area.root,
                });
            }
        }

        let mut groups_by_area: HashMap<DockAreaId, Vec<FloatingGroupId>> = HashMap::new();
        let mut groups_by_z: HashMap<i32, FloatingGroupId> = HashMap::new();

        for (group_id, group) in self.floating_groups.iter() {
            if let Err(reason) = group.validate() {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: match reason {
                        GeometryError::NonFinite => "bounds contain a non-finite value",
                        GeometryError::NegativeDimension => "bounds dimensions must not be negative",
                        GeometryError::NonPositiveDimension => "bounds must have positive dimensions",
                        GeometryError::ArithmeticOverflow => "bounds edges overflow",
                        GeometryError::InconsistentConstraints => "invalid bounds",
                    },
                });
            }

            if let Some(first_id) = groups_by_z.insert(group.z_order, group_id) {
                errors.push(ValidationError::DuplicateFloatingZOrder {
                    first: first_id,
                    second: group_id,
                    z_order: group.z_order,
                });
            }

            let Some(area) = self.dock_areas.get(group.area) else {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "references a missing DockAreaId",
                });
                continue;
            };
            if area.kind != DockAreaKind::Floating {
                errors.push(ValidationError::InvalidFloatingGroup {
                    group: group_id,
                    reason: "must reference a Floating DockArea",
                });
                continue;
            }

            registered_areas.insert(group.area);
            groups_by_area.entry(group.area).or_default().push(group_id);
            if self.nodes.contains_key(area.root) {
                roots.push(area.root);
                if self.profile == Profile::Photoshop {
                    validate_photoshop_floating_root(
                        &self.nodes,
                        group_id,
                        area.root,
                        &mut errors,
                    );
                }
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: None,
                    missing: area.root,
                });
            }
        }

        for (area_id, group_ids) in &groups_by_area {
            if group_ids.len() > 1 {
                errors.push(ValidationError::FloatingAreaReferencedMoreThanOnce {
                    area: *area_id,
                    first: group_ids[0],
                    second: group_ids[1],
                });
            }
        }

        for (area_id, _) in self.dock_areas.iter() {
            if !registered_areas.contains(&area_id) {
                errors.push(ValidationError::UnreferencedDockArea { area: area_id });
            }
        }

        if metrics_are_valid && self.profile == Profile::Photoshop
            && photoshop_main_columns > metrics.max_columns
        {
            errors.push(ValidationError::TooManyColumns {
                actual: photoshop_main_columns,
                max: metrics.max_columns,
            });
        }

        if let Some(flyout) = self.flyout {
            match self.nodes.get(flyout.column) {
                None => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing column node",
                }),
                Some(Node::Column(column)) => {
                    if column.mode != ColumnMode::IconsOnly {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "flyout column must be in IconsOnly mode",
                        });
                    }
                    if !column.groups.contains(&flyout.group) {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "group is not a member of the referenced column",
                        });
                    }
                }
                Some(_) => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "column reference does not point to a Column node",
                }),
            }

            if !self.nodes.contains_key(flyout.group) {
                errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing group node",
                });
            }
        }

        // Count all tree-edge references to find reused children, then mark
        // reachability only from registered area roots. These are deliberately
        // separate passes: cycles and missing links in an orphaned component
        // must still be reported instead of hidden by an early return.
        let mut incoming: HashMap<NodeId, usize> = HashMap::new();
        for root in &roots {
            *incoming.entry(*root).or_default() += 1;
        }
        let mut graph_visited = HashSet::new();
        let mut visiting = HashSet::new();
        for (node_id, _) in self.nodes.iter() {
            inspect_graph(
                &self.nodes,
                node_id,
                None,
                &mut graph_visited,
                &mut visiting,
                &mut incoming,
                &mut errors,
            );
        }

        for (node_id, _) in self.nodes.iter() {
            if let Some(references) = incoming.get(&node_id)
                && *references > 1
            {
                errors.push(ValidationError::MultipleParents {
                    node: node_id,
                    references: *references,
                });
            }
        }

        let mut reachable = HashSet::new();
        for root in &roots {
            mark_reachable(&self.nodes, *root, &mut reachable);
        }
        for (node_id, _) in self.nodes.iter() {
            if !reachable.contains(&node_id) {
                errors.push(ValidationError::UnreachableNode { node: node_id });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Test-only wrapper for the C3 normalization contract. Production mutations
    /// call normalize_in_place on a private candidate inside Layout::apply.
    #[cfg(test)]
    fn normalize(&mut self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut candidate = self.clone();
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(())
    }

    /// Crate-visible entry point for tests in sibling modules. This does not
    /// expose normalization in production builds or to downstream UI code.
    #[cfg(test)]
    pub(crate) fn normalize_for_test(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<(), Vec<ValidationError>> {
        self.normalize(metrics)
    }

    fn normalize_in_place(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<(), Vec<ValidationError>> {
        let existing_errors = self.validate(metrics).err().unwrap_or_default();
        let blockers: Vec<_> = existing_errors
            .into_iter()
            .filter(|error| !is_normalizable_validation_error(error))
            .collect();
        if !blockers.is_empty() {
            return Err(blockers);
        }

        // Main roots are normalized first. An empty main root removes its
        // DockArea and clears the corresponding side; it never leaves an empty
        // invisible area in the arena.
        let main_areas = self.main_areas;
        for (index, area_id) in main_areas.into_iter().enumerate() {
            let Some(area_id) = area_id else {
                continue;
            };
            let root = self
                .dock_areas
                .get(area_id)
                .expect("preflight validation rejects missing main areas")
                .root;

            match self.normalize_node(root) {
                Some(normalized_root) => {
                    self.dock_areas
                        .get_mut(area_id)
                        .expect("main area remains present")
                        .root = normalized_root;
                }
                None => {
                    self.dock_areas.remove(area_id);
                    self.main_areas[index] = None;
                }
            }
        }

        // Floating roots are normalized the same way. Empty floating roots
        // remove both the FloatingGroup and its associated DockArea.
        let floating_ids: Vec<_> = self.floating_groups.keys().collect();
        for group_id in floating_ids {
            let Some(group) = self.floating_groups.get(group_id).copied() else {
                continue;
            };
            let root = self
                .dock_areas
                .get(group.area)
                .expect("preflight validation rejects missing floating areas")
                .root;

            match self.normalize_node(root) {
                Some(normalized_root) => {
                    self.dock_areas
                        .get_mut(group.area)
                        .expect("floating area remains present")
                        .root = normalized_root;
                }
                None => {
                    self.floating_groups.remove(group_id);
                    self.dock_areas.remove(group.area);
                }
            }
        }

        // Flyouts are transient. If normalization removed or changed their
        // target, close the flyout instead of retaining stale runtime handles.
        let keep_flyout = self.flyout.is_some_and(|flyout| {
            matches!(
                self.nodes.get(flyout.column),
                Some(Node::Column(column))
                    if column.mode == ColumnMode::IconsOnly
                        && column.groups.contains(&flyout.group)
                        && self.nodes.contains_key(flyout.group)
            )
        });
        if !keep_flyout {
            self.flyout = None;
        }

        self.validate(metrics)
    }

    /// Returns the normalized representative of a node, or None when the
    /// entire subtree is empty. Preflight validation guarantees that every
    /// referenced node exists and that the graph is acyclic and single-parent.
    fn normalize_node(&mut self, node_id: NodeId) -> Option<NodeId> {
        let node = self.nodes.get(node_id)?.clone();
        match node {
            Node::Panel(_) => Some(node_id),
            Node::Tabs(mut tabs) => {
                let old_children = tabs.children;
                let old_active_index = tabs.active_index;
                let mut kept = Vec::with_capacity(old_children.len());

                for (old_index, child) in old_children.into_iter().enumerate() {
                    if let Some(normalized_child) = self.normalize_node(child) {
                        kept.push((old_index, normalized_child));
                    }
                }

                if kept.is_empty() {
                    self.nodes.remove(node_id);
                    return None;
                }

                tabs.active_index = kept
                    .iter()
                    .position(|(old_index, _)| *old_index == old_active_index)
                    .unwrap_or_else(|| old_active_index.min(kept.len() - 1));
                tabs.children = kept.into_iter().map(|(_, child)| child).collect();
                *self.nodes.get_mut(node_id).expect("node exists during normalization") =
                    Node::Tabs(tabs);
                Some(node_id)
            }
            Node::Split(mut split) => {
                let old_children = split.children;
                let old_ratios = split.ratios;
                let mut kept = Vec::with_capacity(old_children.len());

                for (old_index, child) in old_children.into_iter().enumerate() {
                    if let Some(normalized_child) = self.normalize_node(child) {
                        kept.push((normalized_child, old_ratios.get(old_index).copied()));
                    }
                }

                match kept.len() {
                    0 => {
                        self.nodes.remove(node_id);
                        None
                    }
                    1 => {
                        let (only_child, _) = kept[0];
                        self.nodes.remove(node_id);
                        Some(only_child)
                    }
                    _ => {
                        split.children = kept.iter().map(|(child, _)| *child).collect();
                        split.ratios = normalized_ratios(
                            kept.iter().map(|(_, ratio)| *ratio).collect(),
                        );
                        *self.nodes.get_mut(node_id).expect("node exists during normalization") =
                            Node::Split(split);
                        Some(node_id)
                    }
                }
            }
            Node::Column(mut column) => {
                let old_groups = column.groups;
                let old_ratios = column.group_ratios;
                let mut kept = Vec::with_capacity(old_groups.len());

                for (old_index, group) in old_groups.into_iter().enumerate() {
                    if let Some(normalized_group) = self.normalize_node(group) {
                        kept.push((normalized_group, old_ratios.get(old_index).copied()));
                    }
                }

                if kept.is_empty() {
                    self.nodes.remove(node_id);
                    return None;
                }

                // Deliberately do not collapse a Column that has exactly one group.
                column.groups = kept.iter().map(|(group, _)| *group).collect();
                column.group_ratios =
                    normalized_ratios(kept.iter().map(|(_, ratio)| *ratio).collect());
                *self.nodes.get_mut(node_id).expect("node exists during normalization") =
                    Node::Column(column);
                Some(node_id)
            }
        }
    }

    /// Internal constructor used by pure core factories and loading/migration.
    ///
    /// It builds a complete value rather than exposing mutable arena access to
    /// consumers. Public persistent mutation remains command-driven.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        nodes: SlotMap<NodeId, Node>,
        panel_instances: SlotMap<PanelId, PanelInstance>,
        dock_areas: SlotMap<DockAreaId, DockArea>,
        main_areas: [Option<DockAreaId>; 2],
        floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
        canvas: CanvasState,
        profile: Profile,
        version: u64,
        chrome: ChromeMode,
    ) -> Self {
        Self {
            nodes,
            panel_instances,
            dock_areas,
            main_areas,
            floating_groups,
            last_dock_locations: HashMap::new(),
            canvas,
            profile,
            version,
            chrome,
            flyout: None,
        }
    }
}

fn distribute_heights_with_minimums(
    available_height: f32,
    minimums: &[f32],
    weights: &[f32],
) -> Result<Vec<f32>, CommandError> {
    if !available_height.is_finite() || available_height <= 0.0 {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "available group height must be finite and positive",
        });
    }
    if minimums.len() != weights.len() {
        return Err(CommandError::InvalidValue {
            field: "weights",
            reason: "group weights must match expanded group count",
        });
    }
    if minimums.iter().any(|value| !value.is_finite() || *value <= 0.0)
        || weights.iter().any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(CommandError::InvalidValue {
            field: "weights",
            reason: "group minimums must be positive and weights finite/non-negative",
        });
    }
    if minimums.is_empty() {
        return Ok(Vec::new());
    }

    let minimum_total: f32 = minimums.iter().sum();
    if !minimum_total.is_finite() || minimum_total > available_height {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "available height is below the sum of expanded group minimums",
        });
    }

    let mut result = vec![0.0; minimums.len()];
    let mut active: Vec<usize> = (0..minimums.len()).collect();
    let mut remaining = available_height;
    while !active.is_empty() {
        let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
        let equal_share = weight_sum <= f32::EPSILON;
        let proposed: Vec<(usize, f32)> = active.iter().map(|index| {
            let height = if equal_share {
                remaining / active.len() as f32
            } else {
                remaining * weights[*index] / weight_sum
            };
            (*index, height)
        }).collect();
        let mut constrained = Vec::new();
        for (index, height) in proposed {
            if height < minimums[index] {
                result[index] = minimums[index];
                remaining -= minimums[index];
                constrained.push(index);
            }
        }
        if constrained.is_empty() {
            let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
            let equal_share = weight_sum <= f32::EPSILON;
            for index in &active {
                result[*index] = if equal_share {
                    remaining / active.len() as f32
                } else {
                    remaining * weights[*index] / weight_sum
                };
            }
            break;
        }
        active.retain(|index| !constrained.contains(index));
    }

    if result.iter().any(|height| !height.is_finite() || *height <= 0.0) {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "computed group heights are not finite and positive",
        });
    }
    Ok(result)
}

fn side_index(side: DockSide) -> usize {
    match side {
        DockSide::Left => 0,
        DockSide::Right => 1,
    }
}

fn ratio_issues(ratios: &[f32], expected_len: usize) -> Vec<&'static str> {
    let mut issues = Vec::new();
    if ratios.len() != expected_len {
        issues.push("ratio count must match child/group count");
    }
    if ratios.iter().any(|ratio| !ratio.is_finite() || *ratio < 0.0) {
        issues.push("ratios must be finite and non-negative");
    }
    let sum: f32 = ratios.iter().copied().sum();
    if !sum.is_finite() || (sum - 1.0).abs() > RATIO_TOLERANCE {
        issues.push("ratios must sum to 1.0 within 1e-4");
    }
    issues
}

fn is_normalizable_validation_error(error: &ValidationError) -> bool {
    match error {
        ValidationError::InvalidSplit { reason, .. } => {
            *reason == "must contain at least two children" || is_ratio_error(reason)
        }
        ValidationError::InvalidTabs { reason, .. } => {
            *reason == "must contain at least one child"
                || *reason == "active_index is outside children"
        }
        ValidationError::InvalidColumn { reason, .. } => {
            *reason == "must contain at least one group before normalization"
                || is_ratio_error(reason)
        }
        // In a candidate transaction a move may temporarily create an empty
        // source Column while the destination creates another Column. Allow
        // normalization to remove empty columns first; final validation still
        // rejects any layout that exceeds the limit after cleanup.
        ValidationError::TooManyColumns { .. } => true,
        // Flyout is transient and may be closed if the persistent tree changes.
        ValidationError::InvalidFlyout { .. } => true,
        // All other errors describe broken references, invalid metadata, or
        // profile constraints that normalization must not guess how to repair.
        _ => false,
    }
}

fn is_ratio_error(reason: &str) -> bool {
    matches!(
        reason,
        "ratio count must match child/group count"
            | "ratios must be finite and non-negative"
            | "ratios must sum to 1.0 within 1e-4"
    )
}

fn normalized_ratios(ratios: Vec<Option<f32>>) -> Vec<f32> {
    let len = ratios.len();
    if len == 0 {
        return Vec::new();
    }

    let values: Option<Vec<f32>> = ratios
        .into_iter()
        .collect::<Option<Vec<f32>>>()
        .filter(|values| values.iter().all(|value| value.is_finite() && *value >= 0.0));

    let Some(values) = values else {
        return equal_ratios(len);
    };
    let sum: f32 = values.iter().copied().sum();
    if !sum.is_finite() || sum <= f32::EPSILON {
        return equal_ratios(values.len());
    }

    // Leave already-normalized ratios alone to make repeated normalization
    // exactly idempotent rather than introducing floating-point drift.
    if (sum - 1.0).abs() <= RATIO_TOLERANCE {
        return values;
    }

    let normalized: Vec<f32> = values.iter().map(|value| *value / sum).collect();
    if normalized.iter().all(|value| value.is_finite()) {
        normalized
    } else {
        equal_ratios(values.len())
    }
}

fn equal_ratios(len: usize) -> Vec<f32> {
    if len == 0 {
        return Vec::new();
    }
    vec![1.0 / len as f32; len]
}

fn node_children(node: &Node) -> Vec<NodeId> {
    match node {
        Node::Split(split) => split.children.clone(),
        Node::Tabs(tabs) => tabs.children.clone(),
        Node::Column(column) => column.groups.clone(),
        Node::Panel(_) => Vec::new(),
    }
}

fn inspect_graph(
    nodes: &SlotMap<NodeId, Node>,
    node_id: NodeId,
    parent: Option<NodeId>,
    visited: &mut HashSet<NodeId>,
    visiting: &mut HashSet<NodeId>,
    incoming: &mut HashMap<NodeId, usize>,
    errors: &mut Vec<ValidationError>,
) {
    if !nodes.contains_key(node_id) {
        if let Some(owner) = parent {
            errors.push(ValidationError::MissingNode {
                owner: Some(owner),
                missing: node_id,
            });
        }
        return;
    }
    if visiting.contains(&node_id) {
        errors.push(ValidationError::NodeCycle {
            from: parent.unwrap_or(node_id),
            to: node_id,
        });
        return;
    }
    if visited.contains(&node_id) {
        return;
    }

    visiting.insert(node_id);
    if let Some(node) = nodes.get(node_id) {
        for child in node_children(node) {
            if nodes.contains_key(child) {
                *incoming.entry(child).or_default() += 1;
                inspect_graph(
                    nodes,
                    child,
                    Some(node_id),
                    visited,
                    visiting,
                    incoming,
                    errors,
                );
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: Some(node_id),
                    missing: child,
                });
            }
        }
    }
    visiting.remove(&node_id);
    visited.insert(node_id);
}

fn mark_reachable(
    nodes: &SlotMap<NodeId, Node>,
    node_id: NodeId,
    reachable: &mut HashSet<NodeId>,
) {
    if !nodes.contains_key(node_id) || !reachable.insert(node_id) {
        return;
    }
    if let Some(node) = nodes.get(node_id) {
        for child in node_children(node) {
            mark_reachable(nodes, child, reachable);
        }
    }
}

fn validate_photoshop_main_root(
    nodes: &SlotMap<NodeId, Node>,
    area: DockAreaId,
    root: NodeId,
    errors: &mut Vec<ValidationError>,
) -> usize {
    match nodes.get(root) {
        Some(Node::Column(_)) => 1,
        Some(Node::Split(split)) if split.axis == Axis::X => {
            let mut count = 0;
            let mut invalid_child = false;
            for child in &split.children {
                match nodes.get(*child) {
                    Some(Node::Column(_)) => count += 1,
                    Some(_) => invalid_child = true,
                    None => {}
                }
            }
            if invalid_child {
                errors.push(ValidationError::InvalidMainRoot {
                    area,
                    root,
                    reason: "Split(X) main root children must all be Column nodes",
                });
            }
            count
        }
        Some(Node::Split(_)) => {
            errors.push(ValidationError::InvalidMainRoot {
                area,
                root,
                reason: "main Split must use axis X",
            });
            0
        }
        Some(_) => {
            errors.push(ValidationError::InvalidMainRoot {
                area,
                root,
                reason: "main root must be a Column or Split(X) of Columns",
            });
            0
        }
        None => 0,
    }
}

fn validate_photoshop_floating_root(
    nodes: &SlotMap<NodeId, Node>,
    group: FloatingGroupId,
    root: NodeId,
    errors: &mut Vec<ValidationError>,
) {
    match nodes.get(root) {
        Some(Node::Tabs(_)) => {}
        Some(Node::Split(split)) if split.axis == Axis::Y => {
            let invalid_child = split.children.iter().any(|child| {
                nodes.contains_key(*child) && !matches!(nodes.get(*child), Some(Node::Tabs(_)))
            });
            if invalid_child {
                errors.push(ValidationError::InvalidFloatingRoot {
                    group,
                    root,
                    reason: "Split(Y) floating root children must all be Tabs nodes",
                });
            }
        }
        Some(Node::Split(_)) => errors.push(ValidationError::InvalidFloatingRoot {
            group,
            root,
            reason: "floating stack Split must use axis Y",
        }),
        Some(_) => errors.push(ValidationError::InvalidFloatingRoot {
            group,
            root,
            reason: "floating root must be Tabs or Split(Y) of Tabs",
        }),
        None => {}
    }
}

fn geometry_error_reason(error: GeometryError) -> &'static str {
    match error {
        GeometryError::NonFinite => "bounds contain a non-finite value",
        GeometryError::NegativeDimension => "bounds dimensions must not be negative",
        GeometryError::NonPositiveDimension => "bounds must have positive dimensions",
        GeometryError::ArithmeticOverflow => "bounds arithmetic overflowed",
        GeometryError::InconsistentConstraints => "geometry constraints are inconsistent",
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;
    use crate::{
        Axis, CanvasState, ChromeMode, CommandError, CommandValue, DockArea, DockAreaId,
        DockCommand, DockMetrics, DockSide, DropTarget, FloatingGroup, FloatingGroupId, Node,
        NodeId, Panel, PanelConstraints, PanelId, PanelInstance, PanelKey, Point, Profile, Rect,
        Size, Split, Tabs, ValidationError, DropZone,
    };
    use slotmap::{Key, SlotMap};

    fn descriptors() -> Vec<(PanelKey, PanelConstraints)> {
        ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| {
                (
                    PanelKey::new(key).unwrap(),
                    PanelConstraints::new(
                        Size::new(120.0, 80.0),
                        Size::new(280.0, 220.0),
                        None,
                        false,
                    ),
                )
            })
            .collect()
    }

    fn initial_layout(profile: Profile) -> Layout {
        crate::create_initial_layout(profile, &descriptors(), DockMetrics::default()).unwrap()
    }

    #[test]
    fn new_layout_owns_empty_arenas_and_an_independent_canvas() {
        let layout = Layout::new(Profile::Generic);

        assert_eq!(layout.profile(), Profile::Generic);
        assert_eq!(layout.version(), 0);
        assert_eq!(layout.chrome(), ChromeMode::Visible);
        assert_eq!(layout.flyout(), None);
        assert_eq!(layout.canvas(), &CanvasState::default());
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.panel_instances().count(), 0);
        assert_eq!(layout.dock_areas().count(), 0);
        assert_eq!(layout.floating_groups().count(), 0);
        assert_eq!(layout.main_area(DockSide::Left), None);
        assert_eq!(layout.main_area(DockSide::Right), None);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn clone_preserves_runtime_handles_and_read_only_views() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let root = nodes.insert(Node::Split(Split {
            axis: Axis::Y,
            children: Vec::new(),
            ratios: Vec::new(),
        }));
        let panels = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area = areas.insert(DockArea::main(DockSide::Left, root));
        let floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
        let layout = Layout::from_parts(
            nodes,
            panels,
            areas,
            [Some(area), None],
            floating,
            CanvasState::default(),
            Profile::Generic,
            7,
            ChromeMode::Hidden,
        );
        let cloned = layout.clone();

        assert_eq!(cloned.main_area(DockSide::Left).unwrap().root, root);
        assert_eq!(cloned.version(), 7);
        assert_eq!(cloned.chrome(), ChromeMode::Hidden);
    }

    #[test]
    fn initial_factory_layout_passes_validation() {
        let layout = initial_layout(Profile::Photoshop);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn validation_collects_ratio_and_tabs_errors() {
        let mut layout = initial_layout(Profile::Generic);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
            panic!("factory should create a column");
        };
        column.group_ratios = vec![0.2, 0.2];

        let group_id = column.groups[0];
        let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
            panic!("factory should create Tabs");
        };
        tabs.active_index = tabs.children.len();

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidColumn { reason: "ratios must sum to 1.0 within 1e-4", .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidTabs { reason: "active_index is outside children", .. }
        )));
    }

    #[test]
    fn validation_detects_missing_node_and_cycle_even_in_broken_trees() {
        let mut layout = initial_layout(Profile::Generic);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
            panic!("factory should create a column");
        };
        column.groups.push(NodeId::null());
        column.group_ratios.push(0.0);

        let group_id = column.groups[0];
        let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
            panic!("factory should create Tabs");
        };
        tabs.children.push(group_id);

        let orphan_panel = layout
            .nodes
            .insert(Node::Panel(Panel::new(PanelId::null())));

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::MissingNode { missing, .. } if *missing == NodeId::null()
        )));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::NodeCycle { .. })));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::MultipleParents { node, .. } if *node == group_id
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::UnreachableNode { node } if *node == orphan_panel
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::MissingPanelInstance { node, panel }
                if *node == orphan_panel && *panel == PanelId::null()
        )));
    }

    #[test]
    fn validation_detects_duplicate_panel_placement_and_singleton_keys() {
        let mut layout = initial_layout(Profile::Generic);
        let ids: Vec<_> = layout.nodes.iter().filter_map(|(id, node)| match node {
            Node::Panel(panel) => Some((id, panel.panel_id)),
            _ => None,
        }).collect();
        let (first_node, first_panel) = ids[0];
        let (second_node, second_panel) = ids[1];
        *layout.nodes.get_mut(second_node).unwrap() = Node::Panel(Panel::new(first_panel));

        let duplicate_instance = layout.panel_instances.get_mut(second_panel).unwrap();
        duplicate_instance.key = PanelKey::new("layers").unwrap();
        duplicate_instance.constraints.singleton = true;
        layout.panel_instances.get_mut(first_panel).unwrap().constraints.singleton = true;

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::DuplicatePanelPlacement { panel, .. } if *panel == first_panel
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::DuplicateSingletonPanel { .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::UnplacedPanelInstance { panel } if *panel == second_panel
        )));
        assert_ne!(first_node, second_node);
    }

    #[test]
    fn photoshop_profile_rejects_vertical_main_split() {
        let mut layout = initial_layout(Profile::Photoshop);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let children = match layout.nodes.get(root).unwrap() {
            Node::Column(column) => column.groups.clone(),
            _ => unreachable!(),
        };
        *layout.nodes.get_mut(root).unwrap() = Node::Split(Split {
            axis: Axis::Y,
            children: children.clone(),
            ratios: vec![0.5, 0.5],
        });

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidMainRoot { reason: "main Split must use axis X", .. }
        )));
    }

    #[test]
    fn floating_bounds_and_duplicate_z_order_are_validated() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let root = nodes.insert(Node::Tabs(Tabs {
            children: Vec::new(),
            active_index: 0,
            collapsed: false,
        }));
        let panels = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area_one = areas.insert(DockArea::floating(root));
        let area_two = areas.insert(DockArea::floating(root));
        let mut floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
        let bounds = Rect::new(Point::ZERO, Size::new(0.0, 300.0));
        floating.insert(FloatingGroup::new(area_one, bounds, 1));
        floating.insert(FloatingGroup::new(area_two, Rect::new(Point::ZERO, Size::new(400.0, 300.0)), 1));

        let layout = Layout::from_parts(
            nodes,
            panels,
            areas,
            [None, None],
            floating,
            CanvasState::default(),
            Profile::Generic,
            0,
            ChromeMode::Visible,
        );
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidFloatingGroup { reason: "bounds must have positive dimensions", .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::DuplicateFloatingZOrder { z_order: 1, .. }
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidTabs { .. }
        )));
    }

    #[test]
    fn validation_rejects_invalid_metrics_and_canvas_width() {
        let mut layout = Layout::new(Profile::Generic);
        layout.canvas = CanvasState::new(Some(Rect::new(Point::ZERO, Size::new(200.0, 600.0))));
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidCanvas { reason: "canvas width is below min_canvas_width" }
        )));

        let mut metrics = DockMetrics::default();
        metrics.max_columns = 0;
        let errors = layout.validate(&metrics).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::InvalidMetrics { .. }
        )));
    }

    #[test]
    fn normalize_removes_an_empty_main_root_and_area() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let root = nodes.insert(Node::Tabs(Tabs {
            children: Vec::new(),
            active_index: 0,
            collapsed: false,
        }));
        let panels = SlotMap::<PanelId, PanelInstance>::with_key();
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area = areas.insert(DockArea::main(DockSide::Right, root));

        let mut layout = Layout::from_parts(
            nodes,
            panels,
            areas,
            [None, Some(area)],
            SlotMap::<FloatingGroupId, FloatingGroup>::with_key(),
            CanvasState::default(),
            Profile::Generic,
            0,
            ChromeMode::Visible,
        );

        layout.normalize(&DockMetrics::default()).unwrap();
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.dock_areas().count(), 0);
        assert!(layout.main_area(DockSide::Right).is_none());
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));

        // Running normalization again is safe and does not recreate an area.
        layout.normalize(&DockMetrics::default()).unwrap();
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.dock_areas().count(), 0);
    }

    #[test]
    fn normalize_collapses_a_generic_split_with_one_child() {
        let mut panels = SlotMap::<PanelId, PanelInstance>::with_key();
        let panel_id = panels.insert(PanelInstance::new(
            PanelKey::new("layers").unwrap(),
            descriptors()[0].1,
        ));
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let panel_node = nodes.insert(Node::Panel(Panel::new(panel_id)));
        let tabs_node = nodes.insert(Node::Tabs(Tabs {
            children: vec![panel_node],
            active_index: 0,
            collapsed: false,
        }));
        let split_node = nodes.insert(Node::Split(Split {
            axis: Axis::Y,
            children: vec![tabs_node],
            ratios: vec![1.0],
        }));
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area = areas.insert(DockArea::main(DockSide::Right, split_node));

        let mut layout = Layout::from_parts(
            nodes,
            panels,
            areas,
            [None, Some(area)],
            SlotMap::<FloatingGroupId, FloatingGroup>::with_key(),
            CanvasState::default(),
            Profile::Generic,
            0,
            ChromeMode::Visible,
        );

        layout.normalize(&DockMetrics::default()).unwrap();
        assert!(layout.node(split_node).is_none());
        assert!(matches!(layout.node(tabs_node), Some(Node::Tabs(_))));
        assert_eq!(layout.main_area(DockSide::Right).unwrap().root, tabs_node);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn normalize_removes_empty_groups_but_preserves_single_group_columns() {
        let mut layout = initial_layout(Profile::Generic);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let groups = match layout.node(root).unwrap() {
            Node::Column(column) => column.groups.clone(),
            _ => panic!("factory should create a Column"),
        };
        let empty_group = groups[1];
        let panel_nodes = match layout.node(empty_group).unwrap() {
            Node::Tabs(tabs) => tabs.children.clone(),
            _ => panic!("Column groups should be Tabs"),
        };

        let panel_ids: Vec<_> = panel_nodes
            .iter()
            .filter_map(|node_id| match layout.node(*node_id).unwrap() {
                Node::Panel(panel) => Some((*node_id, panel.panel_id)),
                _ => None,
            })
            .collect();
        for (node_id, panel_id) in panel_ids {
            layout.nodes.remove(node_id);
            layout.panel_instances.remove(panel_id);
        }
        let Node::Tabs(tabs) = layout.nodes.get_mut(empty_group).unwrap() else {
            unreachable!();
        };
        tabs.children.clear();
        tabs.active_index = 0;

        let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
            unreachable!();
        };
        column.group_ratios = vec![0.2, 0.2];

        layout.normalize(&DockMetrics::default()).unwrap();

        let Node::Column(column) = layout.node(root).unwrap() else {
            panic!("Column must not collapse when only one group remains");
        };
        assert_eq!(column.groups, vec![groups[0]]);
        assert_eq!(column.group_ratios, vec![1.0]);
        assert!(layout.node(empty_group).is_none());
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));

        let node_count = layout.nodes().count();
        layout.normalize(&DockMetrics::default()).unwrap();
        assert_eq!(layout.nodes().count(), node_count);
        let Node::Column(column) = layout.node(root).unwrap() else {
            panic!("Column must remain a Column");
        };
        assert_eq!(column.groups, vec![groups[0]]);
        assert_eq!(column.group_ratios, vec![1.0]);
    }

    #[test]
    fn normalize_repairs_ratios_and_clamps_active_tab_index_idempotently() {
        let mut layout = initial_layout(Profile::Generic);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let group_id = match layout.node(root).unwrap() {
            Node::Column(column) => column.groups[0],
            _ => unreachable!(),
        };
        let Node::Column(column) = layout.nodes.get_mut(root).unwrap() else {
            unreachable!();
        };
        column.group_ratios = vec![0.2, 0.2];
        let Node::Tabs(tabs) = layout.nodes.get_mut(group_id).unwrap() else {
            unreachable!();
        };
        tabs.active_index = 99;

        layout.normalize(&DockMetrics::default()).unwrap();
        let Node::Column(column) = layout.node(root).unwrap() else {
            unreachable!();
        };
        assert_eq!(column.group_ratios, vec![0.5, 0.5]);
        let Node::Tabs(tabs) = layout.node(group_id).unwrap() else {
            unreachable!();
        };
        assert_eq!(tabs.active_index, 1);

        layout.normalize(&DockMetrics::default()).unwrap();
        let Node::Column(column) = layout.node(root).unwrap() else {
            unreachable!();
        };
        assert_eq!(column.group_ratios, vec![0.5, 0.5]);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn normalize_refuses_to_hide_missing_links_or_cycles_and_is_transactional() {
        let mut broken_link = initial_layout(Profile::Generic);
        let root = broken_link.main_area(DockSide::Right).unwrap().root;
        let node_count = broken_link.nodes().count();
        let Node::Column(column) = broken_link.nodes.get_mut(root).unwrap() else {
            unreachable!();
        };
        column.groups.push(NodeId::null());
        column.group_ratios.push(0.0);

        let errors = broken_link.normalize(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(error, ValidationError::MissingNode { .. })));
        assert_eq!(broken_link.nodes().count(), node_count);
        let Node::Column(column) = broken_link.node(root).unwrap() else {
            unreachable!();
        };
        assert_eq!(column.groups.last().copied(), Some(NodeId::null()));

        let mut cycle = initial_layout(Profile::Generic);
        let root = cycle.main_area(DockSide::Right).unwrap().root;
        let Node::Column(column) = cycle.nodes.get_mut(root).unwrap() else {
            unreachable!();
        };
        column.groups.push(root);
        column.group_ratios.push(0.0);

        let errors = cycle.normalize(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(error, ValidationError::NodeCycle { .. })));
        let Node::Column(column) = cycle.node(root).unwrap() else {
            unreachable!();
        };
        assert_eq!(column.groups.last().copied(), Some(root));
    }


    #[test]
    fn normalize_removes_an_empty_column_and_its_main_area() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let root = nodes.insert(Node::Column(crate::Column {
            width: DockMetrics::default().default_column_width,
            mode: crate::ColumnMode::Expanded,
            groups: Vec::new(),
            group_ratios: Vec::new(),
        }));
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area = areas.insert(DockArea::main(DockSide::Right, root));
        let mut layout = Layout::from_parts(
            nodes,
            SlotMap::<PanelId, PanelInstance>::with_key(),
            areas,
            [None, Some(area)],
            SlotMap::<FloatingGroupId, FloatingGroup>::with_key(),
            CanvasState::default(),
            Profile::Photoshop,
            0,
            ChromeMode::Visible,
        );

        layout.normalize(&DockMetrics::default()).unwrap();
        assert!(layout.main_area(DockSide::Right).is_none());
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.dock_areas().count(), 0);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn normalize_removes_empty_floating_root_group_and_area() {
        let mut nodes = SlotMap::<NodeId, Node>::with_key();
        let root = nodes.insert(Node::Tabs(Tabs {
            children: Vec::new(),
            active_index: 0,
            collapsed: false,
        }));
        let mut areas = SlotMap::<DockAreaId, DockArea>::with_key();
        let area = areas.insert(DockArea::floating(root));
        let mut floating = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();
        let group = floating.insert(FloatingGroup::new(
            area,
            Rect::new(Point::new(40.0, 50.0), Size::new(400.0, 300.0)),
            1,
        ));
        let mut layout = Layout::from_parts(
            nodes,
            SlotMap::<PanelId, PanelInstance>::with_key(),
            areas,
            [None, None],
            floating,
            CanvasState::default(),
            Profile::Generic,
            0,
            ChromeMode::Visible,
        );

        layout.normalize(&DockMetrics::default()).unwrap();
        assert!(layout.floating_group(group).is_none());
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.dock_areas().count(), 0);
        assert_eq!(layout.floating_groups().count(), 0);
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }


    #[test]
    fn apply_commits_a_successful_change_and_increments_version_once() {
        let metrics = DockMetrics::default();
        let mut layout = initial_layout(Profile::Generic);
        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        let old_width = match layout.node(column_id).unwrap() {
            Node::Column(column) => column.width,
            _ => panic!("initial main root must be a Column"),
        };

        let outcome = layout
            .apply(
                DockCommand::ResizeColumn { column: column_id, width: old_width + 24.0 },
                &metrics,
            )
            .unwrap();

        assert_eq!(layout.version(), 1);
        assert_eq!(outcome.new_version, 1);
        assert!(outcome.changed_nodes.contains(&column_id));
        assert_eq!(outcome.actual_value, Some(CommandValue::ColumnWidth(old_width + 24.0)));
        assert!(outcome.inverse.is_some());
        assert_eq!(layout.validate(&metrics), Ok(()));

        // Repeating the same effective value is a no-op and must not add a version.
        let no_op = layout
            .apply(
                DockCommand::ResizeColumn { column: column_id, width: old_width + 24.0 },
                &metrics,
            )
            .unwrap();
        assert_eq!(layout.version(), 1);
        assert_eq!(no_op.new_version, 1);
        assert!(no_op.changed_nodes.is_empty());
        assert!(no_op.inverse.is_none());
    }

    #[test]
    fn apply_rejection_leaves_layout_and_version_unchanged() {
        let metrics = DockMetrics::default();
        let mut layout = initial_layout(Profile::Generic);
        let before = format!("{layout:?}");
        let before_version = layout.version();

        let error = layout
            .apply(
                DockCommand::ResizeColumn { column: NodeId::null(), width: 300.0 },
                &metrics,
            )
            .unwrap_err();

        assert!(matches!(error, CommandError::MissingNode { .. }));
        assert_eq!(layout.version(), before_version);
        assert_eq!(format!("{layout:?}"), before);
    }


    #[test]
    fn apply_post_operation_validation_failure_rolls_back_the_candidate() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut columns = Vec::new();

        for index in 0..metrics.max_columns {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("column-panel-{index}")).unwrap(),
                descriptors()[0].1,
            );
            let group = builder.tabs(vec![panel], 0, false);
            let column = builder.column(
                metrics.default_column_width,
                crate::ColumnMode::Expanded,
                vec![group],
                vec![1.0],
            );
            columns.push(column);
        }

        let ratios = vec![1.0 / metrics.max_columns as f32; metrics.max_columns];
        let root = builder.split(Axis::X, columns, ratios);
        builder.main_area(DockSide::Right, root);
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        let before = format!("{layout:?}");
        let error = layout
            .apply(
                DockCommand::OpenPanel {
                    key: PanelKey::new("extra-panel").unwrap(),
                    constraints: descriptors()[0].1,
                    target: Some(DropTarget::MainColumn {
                        side: DockSide::Right,
                        index: metrics.max_columns,
                    }),
                },
                &metrics,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            CommandError::InvalidLayout { errors }
                if errors.iter().any(|error| matches!(error, ValidationError::TooManyColumns { .. }))
        ));
        assert_eq!(layout.version(), 0);
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }



    #[test]
    fn apply_vertical_group_split_uses_the_requested_target_groups_ratio() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut target_groups = Vec::new();

        for index in 0..3 {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("target-panel-{index}")).unwrap(),
                descriptors()[0].1,
            );
            target_groups.push(builder.tabs(vec![panel], 0, false));
        }
        let target_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            target_groups.clone(),
            vec![0.2, 0.5, 0.3],
        );
        builder.main_area(DockSide::Right, target_column);

        let (_, source_panel) = builder.panel(
            PanelKey::new("source-panel").unwrap(),
            descriptors()[0].1,
        );
        let source_group = builder.tabs(vec![source_panel], 0, false);
        let source_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, source_column);

        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout
            .apply(
                DockCommand::SplitGroup {
                    group: source_group,
                    target_group: target_groups[1],
                    zone: DropZone::Bottom,
                    ratio: 0.4,
                },
                &metrics,
            )
            .unwrap();

        let Node::Column(column) = layout.node(target_column).unwrap() else {
            panic!("target column must remain a Column");
        };
        assert_eq!(column.groups, vec![target_groups[0], target_groups[1], source_group, target_groups[2]]);
        assert_eq!(column.group_ratios, vec![0.2, 0.3, 0.2, 0.3]);
        assert!(layout.main_area(DockSide::Left).is_none());
        assert_eq!(layout.version(), 1);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn apply_version_overflow_rejects_commit_atomically() {
        let metrics = DockMetrics::default();
        let mut layout = initial_layout(Profile::Generic);
        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        layout.version = u64::MAX;
        let before = format!("{layout:?}");

        let error = layout
            .apply(
                DockCommand::ResizeColumn { column: column_id, width: 320.0 },
                &metrics,
            )
            .unwrap_err();

        assert!(matches!(error, CommandError::InvalidValue { field: "version", .. }));
        assert_eq!(layout.version(), u64::MAX);
        assert_eq!(format!("{layout:?}"), before);
    }

    #[test]
    fn apply_invalid_value_is_rejected_without_panicking_or_mutating_state() {
        let metrics = DockMetrics::default();
        let mut layout = initial_layout(Profile::Generic);
        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        let before = format!("{layout:?}");

        let error = layout
            .apply(
                DockCommand::ResizeColumn { column: column_id, width: f32::NAN },
                &metrics,
            )
            .unwrap_err();

        assert!(matches!(error, CommandError::InvalidValue { field: "width", .. }));
        assert_eq!(layout.version(), 0);
        assert_eq!(format!("{layout:?}"), before);
    }


    #[test]
    fn apply_move_group_reorders_preserving_ratios_and_inverse_restores_order() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Generic);
        let mut groups = Vec::new();

        for index in 0..3 {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("move-group-panel-{index}")).unwrap(),
                descriptors()[0].1,
            );
            groups.push(builder.tabs(vec![panel], 0, false));
        }
        let column_id = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            groups.clone(),
            vec![0.2, 0.5, 0.3],
        );
        builder.main_area(DockSide::Right, column_id);
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        let moved = layout
            .apply(
                DockCommand::MoveGroup {
                    group: groups[0],
                    column: column_id,
                    index: 3,
                },
                &metrics,
            )
            .unwrap();
        assert!(matches!(
            layout.node(column_id),
            Some(Node::Column(column))
                if column.groups == vec![groups[1], groups[2], groups[0]]
                    && column.group_ratios == vec![0.5, 0.3, 0.2]
        ));
        assert_eq!(layout.version(), 1);

        layout.apply(moved.inverse.expect("forward move has a safe inverse"), &metrics).unwrap();
        assert!(matches!(
            layout.node(column_id),
            Some(Node::Column(column))
                if column.groups == groups && column.group_ratios == vec![0.2, 0.5, 0.3]
        ));
        assert_eq!(layout.version(), 2);

        let moved_backwards = layout
            .apply(
                DockCommand::MoveGroup {
                    group: groups[2],
                    column: column_id,
                    index: 0,
                },
                &metrics,
            )
            .unwrap();
        assert!(matches!(
            layout.node(column_id),
            Some(Node::Column(column))
                if column.groups == vec![groups[2], groups[0], groups[1]]
                    && column.group_ratios == vec![0.3, 0.2, 0.5]
        ));
        layout.apply(moved_backwards.inverse.expect("backward move has a safe inverse"), &metrics).unwrap();
        assert!(matches!(
            layout.node(column_id),
            Some(Node::Column(column))
                if column.groups == groups && column.group_ratios == vec![0.2, 0.5, 0.3]
        ));
        assert_eq!(layout.version(), 4);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn apply_drop_of_panel_at_its_current_tab_slot_is_a_no_op() {
        let metrics = DockMetrics::default();
        let mut layout = initial_layout(Profile::Generic);
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let group = match layout.node(root).unwrap() {
            Node::Column(column) => column.groups[0],
            _ => panic!("initial main root must be a Column"),
        };
        let panel_id = match layout.node(group).unwrap() {
            Node::Tabs(tabs) => match layout.node(tabs.children[0]).unwrap() {
                Node::Panel(panel) => panel.panel_id,
                _ => panic!("tab entry must be a Panel"),
            },
            _ => panic!("column group must be Tabs"),
        };
        let before = format!("{layout:?}");

        let outcome = layout
            .apply(
                DockCommand::MovePanel {
                    panel: panel_id,
                    target: DropTarget::Tabs { group, index: 0 },
                },
                &metrics,
            )
            .unwrap();

        assert_eq!(outcome.new_version, 0);
        assert!(outcome.changed_nodes.is_empty());
        assert!(outcome.inverse.is_none());
        assert_eq!(layout.version(), 0);
        assert_eq!(format!("{layout:?}"), before);

        let self_drop = layout
            .apply(
                DockCommand::SplitGroup {
                    group,
                    target_group: group,
                    zone: DropZone::Right,
                    ratio: 0.5,
                },
                &metrics,
            )
            .unwrap();
        assert_eq!(self_drop.new_version, 0);
        assert!(self_drop.changed_nodes.is_empty());
        assert!(self_drop.inverse.is_none());
        assert_eq!(format!("{layout:?}"), before);
    }

    #[test]
    fn apply_transient_flyout_does_not_increment_persistent_version() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, panel) = builder.panel(
            PanelKey::new("layers").unwrap(),
            descriptors()[0].1,
        );
        let group = builder.tabs(vec![panel], 0, false);
        let column = builder.column(
            metrics.rail_width,
            crate::ColumnMode::IconsOnly,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        let outcome = layout
            .apply(DockCommand::OpenFlyout { column, group }, &metrics)
            .unwrap();

        assert_eq!(layout.flyout(), Some(crate::FlyoutState::new(column, group)));
        assert_eq!(layout.version(), 0);
        assert_eq!(outcome.new_version, 0);
        assert!(outcome.inverse.is_none());
        assert!(outcome.changed_nodes.is_empty());
    }


    #[test]
    fn c43_activate_and_reorder_tabs_preserve_active_panel_identity() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Generic);
        let (first_id, first_node) = builder.panel(PanelKey::new("c43-first").unwrap(), descriptors()[0].1);
        let (second_id, second_node) = builder.panel(PanelKey::new("c43-second").unwrap(), descriptors()[0].1);
        let (third_id, third_node) = builder.panel(PanelKey::new("c43-third").unwrap(), descriptors()[0].1);
        let group = builder.tabs(vec![first_node, second_node, third_node], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        layout.apply(DockCommand::ActivateTab { group, panel: second_id }, &metrics).unwrap();
        assert!(matches!(layout.node(group), Some(Node::Tabs(tabs)) if tabs.active_index == 1));
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::ReorderTab { group, panel: second_id, to_index: 2 },
            &metrics,
        ).unwrap();
        assert!(matches!(
            layout.node(group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![first_node, third_node, second_node] && tabs.active_index == 2
        ));
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::ReorderTab { group, panel: first_id, to_index: 1 },
            &metrics,
        ).unwrap();
        assert!(matches!(
            layout.node(group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![third_node, first_node, second_node] && tabs.active_index == 2
        ));
        assert_eq!(layout.panel_instances().count(), 3);
        assert!(layout.panel_instance(third_id).is_some());
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c43_move_panel_between_dock_and_floating_keeps_instance_and_activates_destination() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Generic);
        let (moving_id, moving_node) = builder.panel(PanelKey::new("c43-moving").unwrap(), descriptors()[0].1);
        let (_, dock_sibling) = builder.panel(PanelKey::new("c43-dock-sibling").unwrap(), descriptors()[0].1);
        let dock_group = builder.tabs(vec![moving_node, dock_sibling], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![dock_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);

        let (_, floating_sibling) = builder.panel(PanelKey::new("c43-floating-sibling").unwrap(), descriptors()[0].1);
        let floating_root = builder.tabs(vec![floating_sibling], 0, false);
        let floating_id = builder.floating_group(
            floating_root,
            Rect::new(Point::new(80.0, 70.0), Size::new(360.0, 260.0)),
            1,
        );
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::MovePanel {
                panel: moving_id,
                target: DropTarget::Tabs { group: floating_root, index: 1 },
            },
            &metrics,
        ).unwrap();
        assert!(matches!(
            layout.node(floating_root),
            Some(Node::Tabs(tabs))
                if tabs.children.len() == 2 && tabs.active_index == 1
                    && matches!(layout.node(tabs.children[1]), Some(Node::Panel(panel)) if panel.panel_id == moving_id)
        ));
        assert!(layout.panel_instance(moving_id).is_some());
        assert_eq!(layout.panel_instances().count(), 3);
        assert!(layout.floating_group(floating_id).is_some());
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::MovePanel {
                panel: moving_id,
                target: DropTarget::Tabs { group: dock_group, index: 1 },
            },
            &metrics,
        ).unwrap();
        assert!(matches!(
            layout.node(dock_group),
            Some(Node::Tabs(tabs))
                if tabs.children.len() == 2 && tabs.active_index == 1
                    && matches!(layout.node(tabs.children[1]), Some(Node::Panel(panel)) if panel.panel_id == moving_id)
        ));
        assert!(layout.floating_group(floating_id).is_some());
        assert_eq!(layout.panel_instances().count(), 3);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c43_close_panel_removes_instance_and_empty_group_column_and_remembers_location() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let key = PanelKey::new("c43-last-panel").unwrap();
        let (panel_id, panel_node) = builder.panel(key.clone(), descriptors()[0].1);
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(DockCommand::ClosePanel { panel: panel_id }, &metrics).unwrap();

        assert!(layout.panel_instance(panel_id).is_none());
        assert!(layout.main_area(DockSide::Right).is_none());
        assert_eq!(layout.nodes().count(), 0);
        assert_eq!(layout.panel_instances().count(), 0);
        assert_eq!(
            layout.last_dock_location(&key),
            Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
        );
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c43_close_group_removes_all_owned_panel_instances_without_losing_other_groups() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let key_a = PanelKey::new("c43-group-a").unwrap();
        let key_b = PanelKey::new("c43-group-b").unwrap();
        let key_c = PanelKey::new("c43-group-c").unwrap();
        let (panel_a, node_a) = builder.panel(key_a.clone(), descriptors()[0].1);
        let (panel_b, node_b) = builder.panel(key_b.clone(), descriptors()[0].1);
        let (panel_c, node_c) = builder.panel(key_c, descriptors()[0].1);
        let closing_group = builder.tabs(vec![node_a, node_b], 0, false);
        let kept_group = builder.tabs(vec![node_c], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![closing_group, kept_group],
            vec![0.5, 0.5],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        layout.apply(DockCommand::CloseGroup { group: closing_group }, &metrics).unwrap();

        assert!(layout.panel_instance(panel_a).is_none());
        assert!(layout.panel_instance(panel_b).is_none());
        assert_eq!(layout.panel_instances().count(), 1);
        assert!(layout.panel_instance(panel_c).is_some());
        assert_eq!(
            layout.last_dock_location(&key_a),
            Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
        );
        assert_eq!(
            layout.last_dock_location(&key_b),
            Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
        );
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(value)) if value.groups == vec![kept_group]
        ));
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c43_open_panel_restores_runtime_location_then_falls_back_to_profile_default() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let key = PanelKey::new("c43-restore-location").unwrap();
        let (closed_id, closed_node) = builder.panel(key.clone(), descriptors()[0].1);
        let (_, sibling) = builder.panel(PanelKey::new("c43-restore-sibling").unwrap(), descriptors()[0].1);
        let group = builder.tabs(vec![closed_node, sibling], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        layout.apply(DockCommand::ClosePanel { panel: closed_id }, &metrics).unwrap();
        assert!(layout.panel_instance(closed_id).is_none());
        assert_eq!(
            layout.last_dock_location(&key),
            Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
        );

        layout.apply(
            DockCommand::OpenPanel {
                key: key.clone(),
                constraints: descriptors()[0].1,
                target: None,
            },
            &metrics,
        ).unwrap();
        let active_panel = match layout.node(group).unwrap() {
            Node::Tabs(tabs) => match layout.node(tabs.children[tabs.active_index]).unwrap() {
                Node::Panel(panel) => panel.panel_id,
                other => panic!("restored tab should point to a Panel node: {other:?}"),
            },
            other => panic!("remembered group should remain a Tabs node: {other:?}"),
        };
        assert_ne!(active_panel, closed_id);
        assert_eq!(layout.panel_instances().count(), 2);
        assert_eq!(layout.validate(&metrics), Ok(()));

        let default_key = PanelKey::new("c43-default-placement").unwrap();
        let mut empty = Layout::new(Profile::Photoshop);
        empty.apply(
            DockCommand::OpenPanel {
                key: default_key.clone(),
                constraints: descriptors()[0].1,
                target: None,
            },
            &metrics,
        ).unwrap();
        let default_panel_id = empty.panel_instances().find_map(|(id, instance)| {
            (instance.key == default_key).then_some(id)
        }).unwrap();
        empty.apply(DockCommand::ClosePanel { panel: default_panel_id }, &metrics).unwrap();
        assert!(empty.main_area(DockSide::Right).is_none());
        assert_eq!(
            empty.last_dock_location(&default_key),
            Some(crate::LastDockLocation::new(DockSide::Right, 0, 0))
        );

        // The former group/column no longer exists, so reopening must use the
        // right-dock profile fallback rather than a stale NodeId.
        empty.apply(
            DockCommand::OpenPanel {
                key: default_key,
                constraints: descriptors()[0].1,
                target: None,
            },
            &metrics,
        ).unwrap();
        assert!(empty.main_area(DockSide::Right).is_some());
        assert_eq!(empty.validate(&metrics), Ok(()));
    }

    #[test]
    fn c43_opening_existing_singleton_activates_it_without_toggling_or_validating_new_target() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let singleton_key = PanelKey::new("c43-singleton").unwrap();
        let singleton_constraints = PanelConstraints::new(
            Size::new(80.0, 60.0),
            Size::new(180.0, 120.0),
            None,
            true,
        );
        let (singleton_id, singleton_node) = builder.panel(singleton_key.clone(), singleton_constraints);
        let (_, other_node) = builder.panel(PanelKey::new("c43-other").unwrap(), descriptors()[0].1);
        let group = builder.tabs(vec![singleton_node, other_node], 1, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        let first = layout.apply(
            DockCommand::OpenPanel {
                key: singleton_key.clone(),
                constraints: singleton_constraints,
                target: Some(DropTarget::Tabs { group: NodeId::null(), index: 0 }),
            },
            &metrics,
        ).unwrap();
        assert_eq!(layout.panel_instances().count(), 2);
        assert!(layout.panel_instance(singleton_id).is_some());
        assert!(matches!(layout.node(group), Some(Node::Tabs(tabs)) if tabs.active_index == 0));
        assert_eq!(layout.version(), 1);
        assert!(first.inverse.is_some());
        assert_eq!(layout.validate(&metrics), Ok(()));

        let second = layout.apply(
            DockCommand::OpenPanel {
                key: singleton_key,
                constraints: singleton_constraints,
                target: None,
            },
            &metrics,
        ).unwrap();
        assert_eq!(layout.panel_instances().count(), 2);
        assert_eq!(layout.version(), 1);
        assert_eq!(second.new_version, 1);
        assert!(second.changed_nodes.is_empty());
        assert!(second.inverse.is_none());
        assert_eq!(layout.validate(&metrics), Ok(()));
    }



    #[test]
    fn c45_resize_groups_changes_only_adjacent_pair_and_reports_minimum_clamp() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let constraints = |min_height: f32, max_width: f32| {
            PanelConstraints::new(
                Size::new(120.0, min_height),
                Size::new(200.0, min_height.max(140.0)),
                Some(Size::new(max_width, 600.0)),
                false,
            )
        };
        let (_, panel_a) = builder.panel(PanelKey::new("c45-group-a").unwrap(), constraints(120.0, 500.0));
        let (_, panel_b) = builder.panel(PanelKey::new("c45-group-b").unwrap(), constraints(100.0, 500.0));
        let (_, panel_c) = builder.panel(PanelKey::new("c45-group-c").unwrap(), constraints(250.0, 500.0));
        let group_a = builder.tabs(vec![panel_a], 0, false);
        let group_b = builder.tabs(vec![panel_b], 0, false);
        let group_c = builder.tabs(vec![panel_c], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group_a, group_b, group_c],
            vec![1.0 / 3.0, 0.25, 5.0 / 12.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        let before_ratios = match layout.node(column).unwrap() {
            Node::Column(value) => value.group_ratios.clone(),
            _ => unreachable!(),
        };

        let outcome = layout.apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 500.0,
                available_height: 600.0,
            },
            &metrics,
        ).unwrap();

        assert!(matches!(
            outcome.actual_value,
            Some(CommandValue::GroupBoundaryDelta(delta)) if (delta - 50.0).abs() < 1.0e-3
        ));
        let Node::Column(value) = layout.node(column).unwrap() else {
            panic!("target should remain a Column");
        };
        assert!((value.group_ratios[0] - (5.0 / 12.0)).abs() < 1.0e-4);
        assert!((value.group_ratios[1] - (1.0 / 6.0)).abs() < 1.0e-4);
        assert_eq!(value.group_ratios[2], before_ratios[2]);
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(outcome.inverse.unwrap(), &metrics).unwrap();
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(value)) if value.group_ratios == before_ratios
        ));
        assert_eq!(layout.validate(&metrics), Ok(()));

        let min_clamp = layout.apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: -500.0,
                available_height: 600.0,
            },
            &metrics,
        ).unwrap();
        assert!(matches!(
            min_clamp.actual_value,
            Some(CommandValue::GroupBoundaryDelta(delta)) if (delta + 80.0).abs() < 1.0e-3
        ));
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(value)) if (value.group_ratios[0] - 0.2).abs() < 1.0e-4
        ));
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c45_collapsed_group_has_fixed_height_and_cannot_be_resized_at_its_boundary() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, collapsed_panel) = builder.panel(PanelKey::new("c45-collapsed").unwrap(), descriptors()[0].1);
        let (_, expanded_a_panel) = builder.panel(PanelKey::new("c45-expanded-a").unwrap(), descriptors()[0].1);
        let (_, expanded_b_panel) = builder.panel(PanelKey::new("c45-expanded-b").unwrap(), descriptors()[0].1);
        let collapsed = builder.tabs(vec![collapsed_panel], 0, true);
        let expanded_a = builder.tabs(vec![expanded_a_panel], 0, false);
        let expanded_b = builder.tabs(vec![expanded_b_panel], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![collapsed, expanded_a, expanded_b],
            vec![0.5, 0.25, 0.25],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();
        let before = format!("{layout:?}");

        let fixed_boundary = layout.apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 100.0,
                available_height: 500.0,
            },
            &metrics,
        ).unwrap();
        assert_eq!(fixed_boundary.actual_value, Some(CommandValue::GroupBoundaryDelta(0.0)));
        assert_eq!(fixed_boundary.inverse, None);
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);

        let too_short = layout.apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 0,
                delta: 10.0,
                available_height: 10.0,
            },
            &metrics,
        ).unwrap_err();
        assert!(matches!(
            too_short,
            CommandError::InvalidValue { field: "available_height", .. }
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);

        let expanded_boundary = layout.apply(
            DockCommand::ResizeGroups {
                column,
                boundary_index: 1,
                delta: 10_000.0,
                available_height: 500.0,
            },
            &metrics,
        ).unwrap();
        assert!(matches!(
            expanded_boundary.actual_value,
            Some(CommandValue::GroupBoundaryDelta(delta)) if (delta - 156.0).abs() < 1.0e-3
        ));
        let Node::Column(value) = layout.node(column).unwrap() else {
            panic!("target should remain a Column");
        };
        let expected_weight_delta = 156.0 * 0.5 / 472.0;
        assert_eq!(value.group_ratios[0], 0.5);
        assert!((value.group_ratios[1] - (0.25 + expected_weight_delta)).abs() < 1.0e-4);
        assert!((value.group_ratios[2] - (0.25 - expected_weight_delta)).abs() < 1.0e-4);
        assert!((value.group_ratios.iter().sum::<f32>() - 1.0).abs() < 1.0e-4);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c45_reset_column_ratios_shares_free_height_only_between_expanded_groups() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..4 {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("c45-reset-panel-{index}")).unwrap(),
                descriptors()[0].1,
            );
            groups.push(builder.tabs(vec![panel], 0, index == 0 || index == 2));
        }
        let initial_ratios = vec![0.1, 0.2, 0.3, 0.4];
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            groups,
            initial_ratios.clone(),
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        let outcome = layout.apply(
            DockCommand::ResetColumnRatios {
                column,
                available_height: 500.0,
            },
            &metrics,
        ).unwrap();
        assert_eq!(
            outcome.actual_value,
            Some(CommandValue::GroupRatios(vec![0.0, 0.5, 0.0, 0.5]))
        );
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(value)) if value.group_ratios == vec![0.0, 0.5, 0.0, 0.5]
        ));
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(outcome.inverse.unwrap(), &metrics).unwrap();
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(value)) if value.group_ratios == initial_ratios
        ));
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c45_reset_group_ratios_respects_minimums_when_equal_share_would_be_too_small() {
        let mut metrics = DockMetrics::default();
        metrics.min_group_height = 80.0;
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, tall_panel) = builder.panel(
            PanelKey::new("c45-tall-group").unwrap(),
            PanelConstraints::new(
                Size::new(100.0, 160.0),
                Size::new(180.0, 200.0),
                None,
                false,
            ),
        );
        let (_, short_panel) = builder.panel(
            PanelKey::new("c45-short-group").unwrap(),
            descriptors()[0].1,
        );
        let tall_group = builder.tabs(vec![tall_panel], 0, false);
        let short_group = builder.tabs(vec![short_panel], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![tall_group, short_group],
            vec![0.5, 0.5],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        let outcome = layout.apply(
            DockCommand::ResetColumnRatios {
                column,
                available_height: 240.0,
            },
            &metrics,
        ).unwrap();
        assert_eq!(outcome.actual_value, Some(CommandValue::GroupRatios(vec![2.0 / 3.0, 1.0 / 3.0])));
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c45_resize_column_clamps_min_and_max_and_canvas_gets_remainder() {
        let metrics = DockMetrics::default();
        let constraints = PanelConstraints::new(
            Size::new(240.0, 100.0),
            Size::new(300.0, 180.0),
            Some(Size::new(500.0, 360.0)),
            false,
        );
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, panel) = builder.panel(PanelKey::new("c45-bounded-column").unwrap(), constraints);
        let group = builder.tabs(vec![panel], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        let min_outcome = layout.apply(
            DockCommand::ResizeColumn { column, width: 100.0 },
            &metrics,
        ).unwrap();
        assert_eq!(min_outcome.actual_value, Some(CommandValue::ColumnWidth(240.0)));
        assert!(matches!(layout.node(column), Some(Node::Column(value)) if value.width == 240.0));

        let max_outcome = layout.apply(
            DockCommand::ResizeColumn { column, width: 900.0 },
            &metrics,
        ).unwrap();
        assert_eq!(max_outcome.actual_value, Some(CommandValue::ColumnWidth(500.0)));
        assert!(matches!(layout.node(column), Some(Node::Column(value)) if value.width == 500.0));
        assert_eq!(layout.validate(&metrics), Ok(()));

        let mut canvas_builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, canvas_panel) = canvas_builder.panel(
            PanelKey::new("c45-canvas-column").unwrap(),
            descriptors()[0].1,
        );
        let canvas_group = canvas_builder.tabs(vec![canvas_panel], 0, false);
        let canvas_column = canvas_builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![canvas_group],
            vec![1.0],
        );
        canvas_builder.main_area(DockSide::Left, canvas_column);
        canvas_builder.canvas(CanvasState::new(Some(Rect::new(
            Point::new(metrics.default_column_width, 0.0),
            Size::new(800.0, 600.0),
        ))));
        let mut canvas_layout = canvas_builder.build();
        let canvas_outcome = canvas_layout.apply(
            DockCommand::ResizeColumn { column: canvas_column, width: 900.0 },
            &metrics,
        ).unwrap();
        assert_eq!(canvas_outcome.actual_value, Some(CommandValue::ColumnWidth(760.0)));
        let bounds = canvas_layout.canvas().last_known_bounds().unwrap();
        assert_eq!(bounds.origin.x, 760.0);
        assert_eq!(bounds.size.width, metrics.min_canvas_width);
        assert_eq!(canvas_layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c45_resize_column_rejects_interior_main_column_target() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut columns = Vec::new();
        for index in 0..2 {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("c45-interior-{index}")).unwrap(),
                descriptors()[0].1,
            );
            let group = builder.tabs(vec![panel], 0, false);
            columns.push(builder.column(
                metrics.default_column_width,
                crate::ColumnMode::Expanded,
                vec![group],
                vec![1.0],
            ));
        }
        let root = builder.split(Axis::X, columns.clone(), vec![0.5, 0.5]);
        builder.main_area(DockSide::Right, root);
        let mut layout = builder.build();
        let before = format!("{layout:?}");

        let error = layout.apply(
            DockCommand::ResizeColumn {
                column: columns[1],
                width: 340.0,
            },
            &metrics,
        ).unwrap_err();

        assert!(matches!(error, CommandError::InvalidTarget { .. }));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c45_resize_column_boundary_preserves_pair_sum_and_clamps_min_max() {
        let metrics = DockMetrics::default();
        let left_constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(200.0, 160.0),
            Some(Size::new(300.0, 400.0)),
            false,
        );
        let right_constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(200.0, 160.0),
            Some(Size::new(500.0, 400.0)),
            false,
        );
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, left_panel) = builder.panel(PanelKey::new("c45-boundary-left").unwrap(), left_constraints);
        let (_, right_panel) = builder.panel(PanelKey::new("c45-boundary-right").unwrap(), right_constraints);
        let left_group = builder.tabs(vec![left_panel], 0, false);
        let right_group = builder.tabs(vec![right_panel], 0, false);
        let left_column = builder.column(280.0, crate::ColumnMode::Expanded, vec![left_group], vec![1.0]);
        let right_column = builder.column(280.0, crate::ColumnMode::Expanded, vec![right_group], vec![1.0]);
        let root = builder.split(Axis::X, vec![left_column, right_column], vec![0.5, 0.5]);
        builder.main_area(DockSide::Right, root);
        let mut layout = builder.build();
        let pair_sum = 560.0;

        let max_clamp = layout.apply(
            DockCommand::ResizeColumnBoundary {
                side: DockSide::Right,
                boundary_index: 0,
                delta: 10_000.0,
            },
            &metrics,
        ).unwrap();
        assert_eq!(max_clamp.actual_value, Some(CommandValue::ColumnBoundaryDelta(20.0)));
        let left_width = match layout.node(left_column).unwrap() {
            Node::Column(value) => value.width,
            _ => unreachable!(),
        };
        let right_width = match layout.node(right_column).unwrap() {
            Node::Column(value) => value.width,
            _ => unreachable!(),
        };
        assert_eq!(left_width, 300.0);
        assert_eq!(right_width, 260.0);
        assert!((left_width + right_width - pair_sum).abs() < 1.0e-4);

        let min_clamp = layout.apply(
            DockCommand::ResizeColumnBoundary {
                side: DockSide::Right,
                boundary_index: 0,
                delta: -10_000.0,
            },
            &metrics,
        ).unwrap();
        assert_eq!(min_clamp.actual_value, Some(CommandValue::ColumnBoundaryDelta(-100.0)));
        let left_width = match layout.node(left_column).unwrap() {
            Node::Column(value) => value.width,
            _ => unreachable!(),
        };
        let right_width = match layout.node(right_column).unwrap() {
            Node::Column(value) => value.width,
            _ => unreachable!(),
        };
        assert_eq!(left_width, 200.0);
        assert_eq!(right_width, 360.0);
        assert!((left_width + right_width - pair_sum).abs() < 1.0e-4);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c45_resize_rejects_non_finite_or_non_positive_inputs_atomically() {
        let metrics = DockMetrics::default();
        let mut layout = initial_layout(Profile::Photoshop);
        let column = layout.main_area(DockSide::Right).unwrap().root;
        let before = format!("{layout:?}");

        for width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
            let error = layout.apply(
                DockCommand::ResizeColumn { column, width },
                &metrics,
            ).unwrap_err();
            assert!(matches!(error, CommandError::InvalidValue { field: "width", .. }));
            assert_eq!(format!("{layout:?}"), before);
            assert_eq!(layout.version(), 0);
        }

        for delta in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = layout.apply(
                DockCommand::ResizeGroups {
                    column,
                    boundary_index: 0,
                    delta,
                    available_height: 400.0,
                },
                &metrics,
            ).unwrap_err();
            assert!(matches!(error, CommandError::InvalidValue { field: "delta", .. }));
            assert_eq!(format!("{layout:?}"), before);
            assert_eq!(layout.version(), 0);
        }

        for available_height in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
            let error = layout.apply(
                DockCommand::ResizeGroups {
                    column,
                    boundary_index: 0,
                    delta: 1.0,
                    available_height,
                },
                &metrics,
            ).unwrap_err();
            assert!(matches!(error, CommandError::InvalidValue { field: "available_height", .. }));
            assert_eq!(format!("{layout:?}"), before);
            assert_eq!(layout.version(), 0);
        }
    }

    #[test]
    fn c45_group_resize_rejects_insufficient_available_height_transactionally() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..2 {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("c45-too-short-{index}")).unwrap(),
                PanelConstraints::new(
                    Size::new(120.0, 100.0),
                    Size::new(220.0, 180.0),
                    None,
                    false,
                ),
            );
            groups.push(builder.tabs(vec![panel], 0, false));
        }
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            groups,
            vec![0.5, 0.5],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();
        let before = format!("{layout:?}");

        let error = layout.apply(
            DockCommand::ResetColumnRatios { column, available_height: 150.0 },
            &metrics,
        ).unwrap_err();
        assert!(matches!(error, CommandError::InvalidValue { field: "available_height", .. }));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }


    #[test]
    fn c45_column_boundary_and_ratio_reset_reject_non_finite_or_non_positive_inputs() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_a) = builder.panel(PanelKey::new("c45-invalid-a").unwrap(), descriptors()[0].1);
        let (_, panel_b) = builder.panel(PanelKey::new("c45-invalid-b").unwrap(), descriptors()[0].1);
        let group_a = builder.tabs(vec![panel_a], 0, false);
        let group_b = builder.tabs(vec![panel_b], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![group_a, group_b],
            vec![0.5, 0.5],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();
        let before = format!("{layout:?}");

        for delta in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = layout.apply(
                DockCommand::ResizeColumnBoundary {
                    side: DockSide::Right,
                    boundary_index: 0,
                    delta,
                },
                &metrics,
            ).unwrap_err();
            assert!(matches!(error, CommandError::InvalidValue { field: "delta", .. }));
            assert_eq!(format!("{layout:?}"), before);
            assert_eq!(layout.version(), 0);
        }

        for available_height in [f32::NAN, f32::INFINITY, 0.0, -10.0] {
            let error = layout.apply(
                DockCommand::ResetColumnRatios { column, available_height },
                &metrics,
            ).unwrap_err();
            assert!(matches!(error, CommandError::InvalidValue { field: "available_height", .. }));
            assert_eq!(format!("{layout:?}"), before);
            assert_eq!(layout.version(), 0);
        }
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c44_horizontal_split_creates_a_neighbor_column_with_semantic_order() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

        let (_, source_panel) = builder.panel(PanelKey::new("c44-horizontal-source").unwrap(), descriptors()[0].1);
        let source_group = builder.tabs(vec![source_panel], 0, true);
        let source_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, source_column);

        let (_, target_panel) = builder.panel(PanelKey::new("c44-horizontal-target").unwrap(), descriptors()[0].1);
        let target_group = builder.tabs(vec![target_panel], 0, false);
        let target_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![target_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, target_column);

        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group,
                zone: DropZone::Right,
                ratio: 0.25,
            },
            &metrics,
        ).unwrap();

        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Split(split) = layout.node(root).unwrap() else {
            panic!("horizontal side drop should produce Split(X)");
        };
        assert_eq!(split.axis, Axis::X);
        assert_eq!(split.children.len(), 2);
        assert_eq!(split.children[0], target_column);
        let new_column = split.children[1];
        assert!(matches!(
            layout.node(new_column),
            Some(Node::Column(column)) if column.groups == vec![source_group]
        ));
        assert_eq!(split.ratios, vec![0.75, 0.25]);
        assert!(matches!(
            layout.node(source_group),
            Some(Node::Tabs(tabs)) if tabs.children == vec![source_panel] && tabs.collapsed
        ));
        assert!(layout.main_area(DockSide::Left).is_none());
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c44_group_move_preserves_tabs_state_across_column_floating_and_docking() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

        let (panel_a, node_a) = builder.panel(PanelKey::new("c44-panel-a").unwrap(), descriptors()[0].1);
        let (panel_b, node_b) = builder.panel(PanelKey::new("c44-panel-b").unwrap(), descriptors()[0].1);
        let (panel_c, node_c) = builder.panel(PanelKey::new("c44-panel-c").unwrap(), descriptors()[0].1);
        let source_group = builder.tabs(vec![node_a, node_b, node_c], 1, true);
        let source_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, source_column);

        let (target_panel, target_node) = builder.panel(PanelKey::new("c44-target").unwrap(), descriptors()[0].1);
        let target_group = builder.tabs(vec![target_node], 0, false);
        let target_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![target_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, target_column);

        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        let expected_children = vec![node_a, node_b, node_c];

        layout.apply(
            DockCommand::MoveGroup { group: source_group, column: target_column, index: 1 },
            &metrics,
        ).unwrap();
        assert!(layout.main_area(DockSide::Left).is_none());
        assert!(matches!(
            layout.node(source_group),
            Some(Node::Tabs(tabs))
                if tabs.children == expected_children && tabs.active_index == 1 && tabs.collapsed
        ));
        assert!(matches!(
            layout.node(target_column),
            Some(Node::Column(column)) if column.groups == vec![target_group, source_group]
        ));
        for panel in [panel_a, panel_b, panel_c, target_panel] {
            assert!(layout.panel_instance(panel).is_some());
        }
        assert_eq!(layout.panel_instances().count(), 4);
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::Float {
                node: source_group,
                bounds: Rect::new(Point::new(70.0, 60.0), Size::new(420.0, 300.0)),
            },
            &metrics,
        ).unwrap();
        let floating_id = layout.floating_groups().next().unwrap().0;
        let floating_root = layout.dock_area(layout.floating_group(floating_id).unwrap().area).unwrap().root;
        assert_eq!(floating_root, source_group);
        assert!(matches!(
            layout.node(floating_root),
            Some(Node::Tabs(tabs))
                if tabs.children == expected_children && tabs.active_index == 1 && tabs.collapsed
        ));
        assert_eq!(layout.panel_instances().count(), 4);
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::DockFloating {
                group: floating_id,
                target: DropTarget::MainColumn { side: DockSide::Left, index: 0 },
            },
            &metrics,
        ).unwrap();
        let redocked_column = layout.main_area(DockSide::Left).unwrap().root;
        assert!(matches!(
            layout.node(redocked_column),
            Some(Node::Column(column)) if column.groups == vec![source_group]
        ));
        assert!(matches!(
            layout.node(source_group),
            Some(Node::Tabs(tabs))
                if tabs.children == expected_children && tabs.active_index == 1 && tabs.collapsed
        ));
        for panel in [panel_a, panel_b, panel_c, target_panel] {
            assert!(layout.panel_instance(panel).is_some());
        }
        assert_eq!(layout.panel_instances().count(), 4);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c44_drop_on_existing_neighbor_column_is_a_noop() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, source_panel) = builder.panel(PanelKey::new("c44-noop-source").unwrap(), descriptors()[0].1);
        let source_group = builder.tabs(vec![source_panel], 0, false);
        let source_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group],
            vec![1.0],
        );
        let (_, target_panel) = builder.panel(PanelKey::new("c44-noop-target").unwrap(), descriptors()[0].1);
        let target_group = builder.tabs(vec![target_panel], 0, false);
        let target_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![target_group],
            vec![1.0],
        );
        let root = builder.split(Axis::X, vec![source_column, target_column], vec![0.35, 0.65]);
        builder.main_area(DockSide::Right, root);
        let mut layout = builder.build();
        let before = format!("{layout:?}");

        let outcome = layout.apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group,
                zone: DropZone::Left,
                ratio: 0.5,
            },
            &metrics,
        ).unwrap();

        assert_eq!(outcome.new_version, 0);
        assert!(outcome.changed_nodes.is_empty());
        assert!(outcome.inverse.is_none());
        assert_eq!(layout.version(), 0);
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }


    #[test]
    fn c44_move_group_and_vertical_split_to_existing_neighbor_are_noops() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();

        for index in 0..3 {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("c44-neighbor-noop-{index}")).unwrap(),
                descriptors()[0].1,
            );
            groups.push(builder.tabs(vec![panel], 0, false));
        }
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            groups.clone(),
            vec![0.2, 0.3, 0.5],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();
        let before = format!("{layout:?}");

        let same_slot = layout.apply(
            DockCommand::MoveGroup {
                group: groups[0],
                column,
                index: 1,
            },
            &metrics,
        ).unwrap();
        assert_eq!(same_slot.new_version, 0);
        assert!(same_slot.changed_nodes.is_empty());
        assert!(same_slot.inverse.is_none());
        assert_eq!(format!("{layout:?}"), before);

        let above_neighbor = layout.apply(
            DockCommand::SplitGroup {
                group: groups[0],
                target_group: groups[1],
                zone: DropZone::Top,
                ratio: 0.5,
            },
            &metrics,
        ).unwrap();
        assert_eq!(above_neighbor.new_version, 0);
        assert!(above_neighbor.changed_nodes.is_empty());
        assert!(above_neighbor.inverse.is_none());
        assert_eq!(format!("{layout:?}"), before);

        let below_neighbor = layout.apply(
            DockCommand::SplitGroup {
                group: groups[1],
                target_group: groups[0],
                zone: DropZone::Bottom,
                ratio: 0.5,
            },
            &metrics,
        ).unwrap();
        assert_eq!(below_neighbor.new_version, 0);
        assert!(below_neighbor.changed_nodes.is_empty());
        assert!(below_neighbor.inverse.is_none());
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c44_split_group_moves_whole_tabs_into_an_existing_floating_stack() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

        let (panel_a, node_a) = builder.panel(
            PanelKey::new("c44-floating-source-a").unwrap(),
            descriptors()[0].1,
        );
        let (panel_b, node_b) = builder.panel(
            PanelKey::new("c44-floating-source-b").unwrap(),
            descriptors()[0].1,
        );
        let source_group = builder.tabs(vec![node_a, node_b], 1, true);
        let source_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, source_column);

        let (target_panel, target_node) = builder.panel(
            PanelKey::new("c44-floating-target").unwrap(),
            descriptors()[0].1,
        );
        let target_group = builder.tabs(vec![target_node], 0, false);
        let floating_id = builder.floating_group(
            target_group,
            Rect::new(Point::new(60.0, 70.0), Size::new(400.0, 280.0)),
            1,
        );
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group,
                zone: DropZone::Bottom,
                ratio: 0.35,
            },
            &metrics,
        ).unwrap();

        let floating = layout.floating_group(floating_id).unwrap();
        let root = layout.dock_area(floating.area).unwrap().root;
        let Node::Split(split) = layout.node(root).unwrap() else {
            panic!("dropping below the floating root should create Split(Y)");
        };
        assert_eq!(split.axis, Axis::Y);
        assert_eq!(split.children, vec![target_group, source_group]);
        assert_eq!(split.ratios, vec![0.65, 0.35]);
        assert!(layout.main_area(DockSide::Left).is_none());
        assert!(matches!(
            layout.node(source_group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![node_a, node_b]
                    && tabs.active_index == 1
                    && tabs.collapsed
        ));
        for panel in [panel_a, panel_b, target_panel] {
            assert!(layout.panel_instance(panel).is_some());
        }
        assert_eq!(layout.panel_instances().count(), 3);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c44_drop_on_adjacent_floating_stack_slot_is_a_noop() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

        let (_, top_panel) = builder.panel(
            PanelKey::new("c44-floating-noop-top").unwrap(),
            descriptors()[0].1,
        );
        let top_group = builder.tabs(vec![top_panel], 0, false);
        let (_, bottom_panel) = builder.panel(
            PanelKey::new("c44-floating-noop-bottom").unwrap(),
            descriptors()[0].1,
        );
        let bottom_group = builder.tabs(vec![bottom_panel], 0, false);
        let stack = builder.split(Axis::Y, vec![top_group, bottom_group], vec![0.4, 0.6]);
        builder.floating_group(
            stack,
            Rect::new(Point::new(40.0, 60.0), Size::new(380.0, 260.0)),
            1,
        );
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        let before = format!("{layout:?}");

        let outcome = layout.apply(
            DockCommand::SplitGroup {
                group: top_group,
                target_group: bottom_group,
                zone: DropZone::Top,
                ratio: 0.5,
            },
            &metrics,
        ).unwrap();

        assert_eq!(outcome.new_version, 0);
        assert!(outcome.changed_nodes.is_empty());
        assert!(outcome.inverse.is_none());
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c44_max_column_limit_allows_group_move_when_empty_source_column_collapses() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        let mut columns = Vec::new();
        let mut panel_ids = Vec::new();

        for index in 0..metrics.max_columns {
            let (panel_id, panel) = builder.panel(
                PanelKey::new(format!("c44-cap-panel-{index}")).unwrap(),
                descriptors()[0].1,
            );
            let group = builder.tabs(vec![panel], 0, index == 0);
            let column = builder.column(
                metrics.default_column_width,
                crate::ColumnMode::Expanded,
                vec![group],
                vec![1.0],
            );
            groups.push(group);
            columns.push(column);
            panel_ids.push(panel_id);
        }
        let root = builder.split(Axis::X, columns.clone(), vec![1.0 / metrics.max_columns as f32; metrics.max_columns]);
        builder.main_area(DockSide::Right, root);
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::SplitGroup {
                group: groups[0],
                target_group: groups[2],
                zone: DropZone::Right,
                ratio: 0.5,
            },
            &metrics,
        ).unwrap();

        let root = layout.main_area(DockSide::Right).unwrap().root;
        let Node::Split(split) = layout.node(root).unwrap() else {
            panic!("main dock should remain a horizontal split");
        };
        assert_eq!(split.children.len(), metrics.max_columns);
        assert!(!split.children.contains(&columns[0]));
        assert!(split.children.iter().any(|column_id| {
            matches!(layout.node(*column_id), Some(Node::Column(column)) if column.groups == vec![groups[0]])
        }));
        assert!(matches!(
            layout.node(groups[0]),
            Some(Node::Tabs(tabs)) if tabs.children.len() == 1 && tabs.collapsed
        ));
        assert_eq!(layout.panel_instances().count(), metrics.max_columns);
        for panel in panel_ids {
            assert!(layout.panel_instance(panel).is_some());
        }
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c44_stale_horizontal_drop_at_column_limit_is_rejected_atomically() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut columns = Vec::new();
        let mut target_groups = Vec::new();

        let (_, source_panel) = builder.panel(
            PanelKey::new("c44-stale-source").unwrap(),
            descriptors()[0].1,
        );
        let source_group = builder.tabs(vec![source_panel], 0, false);

        let (_, sibling_panel) = builder.panel(
            PanelKey::new("c44-stale-sibling").unwrap(),
            descriptors()[0].1,
        );
        let sibling_group = builder.tabs(vec![sibling_panel], 0, false);
        let first_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group, sibling_group],
            vec![0.5, 0.5],
        );
        columns.push(first_column);

        for index in 1..metrics.max_columns {
            let (_, panel) = builder.panel(
                PanelKey::new(format!("c44-full-panel-{index}")).unwrap(),
                descriptors()[0].1,
            );
            let group = builder.tabs(vec![panel], 0, false);
            let column = builder.column(
                metrics.default_column_width,
                crate::ColumnMode::Expanded,
                vec![group],
                vec![1.0],
            );
            target_groups.push(group);
            columns.push(column);
        }

        let root = builder.split(
            Axis::X,
            columns.clone(),
            vec![1.0 / metrics.max_columns as f32; metrics.max_columns],
        );
        builder.main_area(DockSide::Right, root);

        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        let before = format!("{layout:?}");

        // The source column retains sibling_group, so this horizontal move
        // would leave max_columns + 1 nonempty columns in the final layout.
        let error = layout.apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group: target_groups[0],
                zone: DropZone::Left,
                ratio: 0.5,
            },
            &metrics,
        ).unwrap_err();

        assert!(matches!(
            error,
            CommandError::InvalidLayout { errors }
                if errors.iter().any(|error| matches!(
                    error,
                    ValidationError::TooManyColumns { actual, max }
                        if *actual > *max
                ))
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }


    #[test]
    fn c46_float_panel_keeps_panel_instance_and_id() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (moving_panel_id, moving_panel_node) = builder.panel(
            PanelKey::new("c46-float-panel").unwrap(),
            descriptors()[0].1,
        );
        let (_, sibling_panel_node) = builder.panel(
            PanelKey::new("c46-float-sibling").unwrap(),
            descriptors()[0].1,
        );
        let source_group = builder.tabs(vec![moving_panel_node, sibling_panel_node], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::Float {
                node: moving_panel_node,
                bounds: Rect::new(Point::new(80.0, 60.0), Size::new(320.0, 220.0)),
            },
            &metrics,
        ).unwrap();

        let (floating_id, floating) = layout.floating_groups().next().unwrap();
        let floating_root = layout.dock_area(floating.area).unwrap().root;
        assert!(matches!(
            layout.node(floating_root),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![moving_panel_node]
                    && tabs.active_index == 0
                    && !tabs.collapsed
        ));
        assert!(matches!(
            layout.node(moving_panel_node),
            Some(Node::Panel(panel)) if panel.panel_id == moving_panel_id
        ));
        assert!(layout.panel_instance(moving_panel_id).is_some());
        assert_eq!(layout.floating_group(floating_id).unwrap().bounds.size, Size::new(320.0, 220.0));
        assert!(matches!(
            layout.node(source_group),
            Some(Node::Tabs(tabs)) if tabs.children == vec![sibling_panel_node]
        ));
        assert_eq!(layout.panel_instances().count(), 2);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_move_resize_raise_and_center_dock_floating_preserve_source_active_tab() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (dock_panel, dock_panel_node) = builder.panel(
            PanelKey::new("c46-center-dock").unwrap(),
            descriptors()[0].1,
        );
        let dock_group = builder.tabs(vec![dock_panel_node], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![dock_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);

        let (source_panel_a, source_node_a) = builder.panel(
            PanelKey::new("c46-floating-a").unwrap(),
            descriptors()[0].1,
        );
        let (source_panel_b, source_node_b) = builder.panel(
            PanelKey::new("c46-floating-b").unwrap(),
            descriptors()[0].1,
        );
        let source_tabs = builder.tabs(vec![source_node_a, source_node_b], 1, true);
        let source_floating = builder.floating_group(
            source_tabs,
            Rect::new(Point::new(20.0, 30.0), Size::new(420.0, 300.0)),
            1,
        );

        let (_, other_panel_node) = builder.panel(
            PanelKey::new("c46-other-floating").unwrap(),
            descriptors()[0].1,
        );
        let other_tabs = builder.tabs(vec![other_panel_node], 0, false);
        let other_floating = builder.floating_group(
            other_tabs,
            Rect::new(Point::new(50.0, 60.0), Size::new(360.0, 240.0)),
            2,
        );

        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        let moved = layout.apply(
            DockCommand::MoveFloating {
                group: source_floating,
                origin: Point::new(140.0, 150.0),
            },
            &metrics,
        ).unwrap();
        assert_eq!(
            layout.floating_group(source_floating).unwrap().bounds.origin,
            Point::new(140.0, 150.0)
        );
        layout.apply(moved.inverse.unwrap(), &metrics).unwrap();
        assert_eq!(
            layout.floating_group(source_floating).unwrap().bounds.origin,
            Point::new(20.0, 30.0)
        );

        let resized = layout.apply(
            DockCommand::ResizeFloating {
                group: source_floating,
                size: Size::new(10.0, 10.0),
            },
            &metrics,
        ).unwrap();
        assert_eq!(
            resized.actual_value,
            Some(CommandValue::FloatingSize(Size::new(120.0, 80.0)))
        );
        assert_eq!(
            layout.floating_group(source_floating).unwrap().bounds.size,
            Size::new(120.0, 80.0)
        );
        layout.apply(resized.inverse.unwrap(), &metrics).unwrap();
        assert_eq!(
            layout.floating_group(source_floating).unwrap().bounds.size,
            Size::new(420.0, 300.0)
        );

        layout.apply(DockCommand::RaiseFloating { group: source_floating }, &metrics).unwrap();
        assert_eq!(layout.floating_group(source_floating).unwrap().z_order, 3);
        assert_eq!(layout.floating_group(other_floating).unwrap().z_order, 2);
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::DockFloating {
                group: source_floating,
                target: DropTarget::Tabs { group: dock_group, index: 1 },
            },
            &metrics,
        ).unwrap();
        assert!(layout.floating_group(source_floating).is_none());
        assert!(layout.floating_group(other_floating).is_some());
        assert!(matches!(
            layout.node(dock_group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![dock_panel_node, source_node_a, source_node_b]
                    && tabs.active_index == 2
        ));
        for panel in [dock_panel, source_panel_a, source_panel_b] {
            assert!(layout.panel_instance(panel).is_some());
        }
        assert_eq!(layout.panel_instances().count(), 4);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_move_one_panel_between_groups_inside_floating_stack() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (moving_panel, moving_node) = builder.panel(
            PanelKey::new("c46-stack-moving").unwrap(),
            descriptors()[0].1,
        );
        let (remaining_panel, remaining_node) = builder.panel(
            PanelKey::new("c46-stack-remaining").unwrap(),
            descriptors()[0].1,
        );
        let source_group = builder.tabs(vec![moving_node, remaining_node], 0, false);
        let (target_panel, target_node) = builder.panel(
            PanelKey::new("c46-stack-target").unwrap(),
            descriptors()[0].1,
        );
        let target_group = builder.tabs(vec![target_node], 0, false);
        let stack = builder.split(Axis::Y, vec![source_group, target_group], vec![0.6, 0.4]);
        let floating_id = builder.floating_group(
            stack,
            Rect::new(Point::new(40.0, 50.0), Size::new(440.0, 360.0)),
            1,
        );
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::Tabs { group: target_group, index: 1 },
            },
            &metrics,
        ).unwrap();

        assert!(matches!(
            layout.node(source_group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![remaining_node] && tabs.active_index == 0
        ));
        assert!(matches!(
            layout.node(target_group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![target_node, moving_node] && tabs.active_index == 1
        ));
        assert!(matches!(
            layout.node(moving_node),
            Some(Node::Panel(panel)) if panel.panel_id == moving_panel
        ));
        for panel in [moving_panel, remaining_panel, target_panel] {
            assert!(layout.panel_instance(panel).is_some());
        }
        let floating = layout.floating_group(floating_id).unwrap();
        assert!(matches!(
            layout.node(layout.dock_area(floating.area).unwrap().root),
            Some(Node::Split(split)) if split.axis == Axis::Y && split.children == vec![source_group, target_group]
        ));
        assert_eq!(layout.panel_instances().count(), 3);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_move_whole_tabs_group_to_top_of_floating_stack_preserves_tabs_state() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (moving_panel_a, moving_node_a) = builder.panel(
            PanelKey::new("c46-whole-a").unwrap(),
            descriptors()[0].1,
        );
        let (moving_panel_b, moving_node_b) = builder.panel(
            PanelKey::new("c46-whole-b").unwrap(),
            descriptors()[0].1,
        );
        let moving_group = builder.tabs(vec![moving_node_a, moving_node_b], 1, true);
        let main_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![moving_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, main_column);

        let (target_panel, target_node) = builder.panel(
            PanelKey::new("c46-stack-top-target").unwrap(),
            descriptors()[0].1,
        );
        let target_group = builder.tabs(vec![target_node], 0, false);
        let floating_id = builder.floating_group(
            target_group,
            Rect::new(Point::new(80.0, 90.0), Size::new(420.0, 320.0)),
            1,
        );
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::SplitGroup {
                group: moving_group,
                target_group,
                zone: DropZone::Top,
                ratio: 0.3,
            },
            &metrics,
        ).unwrap();

        let floating = layout.floating_group(floating_id).unwrap();
        let root = layout.dock_area(floating.area).unwrap().root;
        assert!(matches!(
            layout.node(root),
            Some(Node::Split(split))
                if split.axis == Axis::Y
                    && split.children == vec![moving_group, target_group]
                    && split.ratios == vec![0.3, 0.7]
        ));
        assert!(matches!(
            layout.node(moving_group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![moving_node_a, moving_node_b]
                    && tabs.active_index == 1
                    && tabs.collapsed
        ));
        for panel in [moving_panel_a, moving_panel_b, target_panel] {
            assert!(layout.panel_instance(panel).is_some());
        }
        assert!(layout.main_area(DockSide::Left).is_none());
        assert_eq!(layout.panel_instances().count(), 3);
        assert_eq!(layout.validate(&metrics), Ok(()));

        // Docking the whole floating stack creates a Column without flattening
        // the Tabs groups or losing their order and panel instances.
        layout.apply(
            DockCommand::DockFloating {
                group: floating_id,
                target: DropTarget::MainColumn { side: DockSide::Left, index: 0 },
            },
            &metrics,
        ).unwrap();
        assert!(layout.floating_group(floating_id).is_none());
        let docked_column = layout.main_area(DockSide::Left).unwrap().root;
        assert!(matches!(
            layout.node(docked_column),
            Some(Node::Column(column))
                if column.groups == vec![moving_group, target_group]
                    && column.group_ratios.len() == 2
                    && (column.group_ratios[0] - 0.3).abs() < 1.0e-4
                    && (column.group_ratios[1] - 0.7).abs() < 1.0e-4
        ));
        assert!(matches!(
            layout.node(moving_group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![moving_node_a, moving_node_b]
                    && tabs.active_index == 1
                    && tabs.collapsed
        ));
        assert_eq!(layout.panel_instances().count(), 3);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_photoshop_rejects_horizontal_split_inside_floating_stack_atomically() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, source_panel) = builder.panel(
            PanelKey::new("c46-horizontal-source").unwrap(),
            descriptors()[0].1,
        );
        let source_group = builder.tabs(vec![source_panel], 0, false);
        let source_column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![source_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, source_column);

        let (_, top_panel) = builder.panel(
            PanelKey::new("c46-horizontal-top").unwrap(),
            descriptors()[0].1,
        );
        let top_group = builder.tabs(vec![top_panel], 0, false);
        let (_, bottom_panel) = builder.panel(
            PanelKey::new("c46-horizontal-bottom").unwrap(),
            descriptors()[0].1,
        );
        let bottom_group = builder.tabs(vec![bottom_panel], 0, false);
        let stack = builder.split(Axis::Y, vec![top_group, bottom_group], vec![0.5, 0.5]);
        builder.floating_group(
            stack,
            Rect::new(Point::new(60.0, 70.0), Size::new(440.0, 340.0)),
            1,
        );
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        let before = format!("{layout:?}");

        let error = layout.apply(
            DockCommand::SplitGroup {
                group: source_group,
                target_group: top_group,
                zone: DropZone::Left,
                ratio: 0.5,
            },
            &metrics,
        ).unwrap_err();

        assert!(matches!(
            error,
            CommandError::InvalidTarget { reason: "horizontal splits are not supported inside a floating stack" }
        ));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }



    #[test]
    fn c46_successful_move_of_last_panel_cleans_up_floating_area() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

        let (dock_panel, dock_panel_node) = builder.panel(
            PanelKey::new("c46-cleanup-dock").unwrap(),
            descriptors()[0].1,
        );
        let dock_group = builder.tabs(vec![dock_panel_node], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![dock_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);

        let (floating_panel, floating_panel_node) = builder.panel(
            PanelKey::new("c46-cleanup-floating").unwrap(),
            descriptors()[0].1,
        );
        let floating_root = builder.tabs(vec![floating_panel_node], 0, false);
        let floating_id = builder.floating_group(
            floating_root,
            Rect::new(Point::new(60.0, 70.0), Size::new(360.0, 260.0)),
            1,
        );
        let mut layout = builder.build();
        let area_id = layout.floating_group(floating_id).unwrap().area;
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::MovePanel {
                panel: floating_panel,
                target: DropTarget::Tabs { group: dock_group, index: 1 },
            },
            &metrics,
        ).unwrap();

        assert!(layout.floating_group(floating_id).is_none());
        assert!(layout.dock_area(area_id).is_none());
        assert!(layout.node(floating_root).is_none());
        assert!(matches!(
            layout.node(dock_group),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![dock_panel_node, floating_panel_node]
                    && tabs.active_index == 1
        ));
        assert!(layout.panel_instance(floating_panel).is_some());
        assert!(layout.panel_instance(dock_panel).is_some());
        assert_eq!(layout.panel_instances().count(), 2);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_failed_dock_preserves_the_floating_group_and_area_atomically() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, dock_panel_node) = builder.panel(
            PanelKey::new("c46-failed-dock-target").unwrap(),
            descriptors()[0].1,
        );
        let dock_group = builder.tabs(vec![dock_panel_node], 0, false);
        let column = builder.column(
            metrics.default_column_width,
            crate::ColumnMode::Expanded,
            vec![dock_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, column);
        let (floating_panel, floating_panel_node) = builder.panel(
            PanelKey::new("c46-failed-dock-source").unwrap(),
            descriptors()[0].1,
        );
        let floating_root = builder.tabs(vec![floating_panel_node], 0, false);
        let floating_id = builder.floating_group(
            floating_root,
            Rect::new(Point::new(90.0, 80.0), Size::new(420.0, 280.0)),
            2,
        );
        let mut layout = builder.build();
        let area_id = layout.floating_group(floating_id).unwrap().area;
        let before = format!("{layout:?}");
        assert_eq!(layout.validate(&metrics), Ok(()));

        let error = layout.apply(
            DockCommand::DockFloating {
                group: floating_id,
                target: DropTarget::Tabs { group: NodeId::null(), index: 0 },
            },
            &metrics,
        ).unwrap_err();

        assert!(matches!(error, CommandError::MissingNode { node } if node == NodeId::null()));
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), 0);
        assert!(layout.floating_group(floating_id).is_some());
        assert!(layout.dock_area(area_id).is_some());
        assert!(matches!(
            layout.node(floating_root),
            Some(Node::Tabs(tabs)) if tabs.children == vec![floating_panel_node]
        ));
        assert!(layout.panel_instance(floating_panel).is_some());
        assert_eq!(layout.panel_instances().count(), 2);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_floating_bounds_are_model_space_and_are_not_screen_clamped() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(
            PanelKey::new("c46-offscreen").unwrap(),
            descriptors()[0].1,
        );
        let tabs = builder.tabs(vec![panel_node], 0, false);
        let floating_id = builder.floating_group(
            tabs,
            Rect::new(Point::new(-9000.0, -7000.0), Size::new(360.0, 260.0)),
            1,
        );
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        assert_eq!(
            layout.floating_group(floating_id).unwrap().bounds.origin,
            Point::new(-9000.0, -7000.0)
        );

        let moved = layout.apply(
            DockCommand::MoveFloating {
                group: floating_id,
                origin: Point::new(-12000.0, 3000.0),
            },
            &metrics,
        ).unwrap();
        assert_eq!(
            layout.floating_group(floating_id).unwrap().bounds.origin,
            Point::new(-12000.0, 3000.0)
        );
        layout.apply(moved.inverse.unwrap(), &metrics).unwrap();
        assert_eq!(
            layout.floating_group(floating_id).unwrap().bounds.origin,
            Point::new(-9000.0, -7000.0)
        );
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_paint_and_hit_test_orders_are_inverse_views_of_z_order() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);
        let mut ids = Vec::new();
        for (key, z_order) in [
            ("c46-z-high", 9),
            ("c46-z-low", 1),
            ("c46-z-mid", 4),
        ] {
            let (_, panel_node) = builder.panel(PanelKey::new(key).unwrap(), descriptors()[0].1);
            let tabs = builder.tabs(vec![panel_node], 0, false);
            ids.push(builder.floating_group(
                tabs,
                Rect::new(Point::new(20.0 * z_order as f32, 30.0), Size::new(300.0, 220.0)),
                z_order,
            ));
        }
        let mut layout = builder.build();
        assert_eq!(layout.validate(&metrics), Ok(()));
        let [high, low, mid] = [ids[0], ids[1], ids[2]];

        assert_eq!(
            layout.floating_groups_in_paint_order().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![low, mid, high]
        );
        assert_eq!(
            layout.floating_groups_in_hit_test_order().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![high, mid, low]
        );

        layout.apply(DockCommand::RaiseFloating { group: low }, &metrics).unwrap();
        assert_eq!(layout.floating_group(low).unwrap().z_order, 10);
        assert_eq!(
            layout.floating_groups_in_paint_order().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![mid, high, low]
        );
        assert_eq!(
            layout.floating_groups_in_hit_test_order().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![low, high, mid]
        );
        assert_eq!(layout.validate(&metrics), Ok(()));
    }

    #[test]
    fn c46_center_merge_between_floating_windows_keeps_panel_instances_and_cleans_source() {
        let metrics = DockMetrics::default();
        let mut builder = crate::layout_builder::LayoutBuilder::new(Profile::Photoshop);

        let (target_panel, target_node) = builder.panel(
            PanelKey::new("c46-floating-merge-target").unwrap(),
            descriptors()[0].1,
        );
        let target_tabs = builder.tabs(vec![target_node], 0, false);
        let target_floating = builder.floating_group(
            target_tabs,
            Rect::new(Point::new(100.0, 120.0), Size::new(420.0, 300.0)),
            1,
        );

        let (source_panel_a, source_node_a) = builder.panel(
            PanelKey::new("c46-floating-merge-a").unwrap(),
            descriptors()[0].1,
        );
        let (source_panel_b, source_node_b) = builder.panel(
            PanelKey::new("c46-floating-merge-b").unwrap(),
            descriptors()[0].1,
        );
        let source_tabs = builder.tabs(vec![source_node_a, source_node_b], 1, true);
        let source_floating = builder.floating_group(
            source_tabs,
            Rect::new(Point::new(160.0, 180.0), Size::new(400.0, 280.0)),
            2,
        );
        let mut layout = builder.build();
        let source_area = layout.floating_group(source_floating).unwrap().area;
        assert_eq!(layout.validate(&metrics), Ok(()));

        layout.apply(
            DockCommand::DockFloating {
                group: source_floating,
                target: DropTarget::Tabs { group: target_tabs, index: 1 },
            },
            &metrics,
        ).unwrap();

        assert!(layout.floating_group(source_floating).is_none());
        assert!(layout.dock_area(source_area).is_none());
        assert!(layout.floating_group(target_floating).is_some());
        assert!(matches!(
            layout.node(target_tabs),
            Some(Node::Tabs(tabs))
                if tabs.children == vec![target_node, source_node_a, source_node_b]
                    && tabs.active_index == 2
                    && !tabs.collapsed
        ));
        for panel in [target_panel, source_panel_a, source_panel_b] {
            assert!(layout.panel_instance(panel).is_some());
        }
        assert_eq!(layout.panel_instances().count(), 3);
        assert_eq!(layout.validate(&metrics), Ok(()));
    }


}
