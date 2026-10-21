use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    Axis, CanvasState, ChromeMode, ColumnMode, CommandError, CommandOutcome, CommandValue,
    DockArea, DockAreaId, DockAreaKind, DockCommand, DockMetrics, DockSide, DropTarget, DropZone,
    FloatingGroup, FloatingGroupId, FlyoutState, GeometryError, Node, NodeId, PanelId, PanelInstance,
    Profile, Rect, Size, ValidationError,
};

const RATIO_TOLERANCE: f32 = 1.0e-4;
const PIXEL_TOLERANCE: f32 = 1.0e-3;

/// Complete UI-independent docking state.
///
/// Layout is the sole owner of runtime arenas. Public access is read-only;
/// persistent edits will be performed through DockCommand once the command
/// layer is introduced. Flyout state is transient and kept separately from the
/// persistent tree.
#[derive(Clone, Debug)]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panel_instances: SlotMap<PanelId, PanelInstance>,
    dock_areas: SlotMap<DockAreaId, DockArea>,
    main_areas: [Option<DockAreaId>; 2],
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
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
        self.validate(metrics).map_err(CommandError::InvalidLayout)?;

        let mut candidate = self.clone();
        let applied = candidate.apply_command(command, metrics)?;
        candidate.normalize_in_place(metrics).map_err(CommandError::InvalidLayout)?;

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
                self.validate_drop_target(*target, metrics)?;

                if let Some((existing_id, _)) = self.panel_instances.iter().find(|(_, instance)| {
                    instance.key == *key && (constraints.singleton || instance.constraints.singleton)
                }) {
                    let panel_node = self.find_panel_node(existing_id).ok_or(
                        CommandError::MissingPanel { panel: existing_id },
                    )?;
                    let (group, _) = self.tab_parent_index(panel_node).ok_or(
                        CommandError::InvalidTarget {
                            reason: "existing singleton panel is not inside a Tabs node",
                        },
                    )?;
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

                let panel_id = self.panel_instances.insert(PanelInstance::new(key.clone(), *constraints));
                let panel_node = self.nodes.insert(Node::Panel(crate::Panel::new(panel_id)));
                self.insert_node_for_target(panel_node, *target, metrics, Some(0.5))?;
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
                self.detach_node(panel_node)?;
                self.insert_node_for_target(panel_node, *target, metrics, Some(0.5))?;
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
                        self.detach_node(*group)?;
                        let new_len = match self.nodes.get(*column) {
                            Some(Node::Column(value)) => value.groups.len(),
                            _ => return Err(CommandError::InvalidTarget { reason: "MoveGroup target changed unexpectedly" }),
                        };
                        self.insert_group_at_column(*column, *group, adjusted_index.min(new_len), None)?;
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
                self.detach_node(*group)?;
                let new_len = match self.nodes.get(*column) {
                    Some(Node::Column(value)) => value.groups.len(),
                    _ => return Err(CommandError::InvalidTarget { reason: "MoveGroup target changed unexpectedly" }),
                };
                self.insert_group_at_column(*column, *group, (*index).min(new_len), None)?;
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
                let source_column = self.find_owner_column(*group);
                let source_index = source_column.and_then(|column| match self.nodes.get(column) {
                    Some(Node::Column(column_data)) if column_data.groups.len() > 1 => {
                        column_data.groups.iter().position(|id| id == group).map(|index| (column, index))
                    }
                    _ => None,
                });
                self.detach_node(*group)?;
                self.insert_group_edge(*group, *target_group, *zone, Some(*ratio), metrics)?;
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
            DockCommand::ResizeGroups { column, ratios } => {
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.group_ratios.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "ResizeGroups target must be a Column node" }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                let group_count = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.groups.len(),
                    _ => 0,
                };
                if ratios.len() != group_count {
                    return Err(CommandError::InvalidValue { field: "ratios", reason: "ratio count must match group count" });
                }
                if let Some(reason) = ratio_issues(ratios, group_count).first() {
                    return Err(CommandError::InvalidValue { field: "ratios", reason: *reason });
                }
                if current == *ratios {
                    return Ok(CommandApplication::persistent());
                }
                if let Some(Node::Column(column)) = self.nodes.get_mut(*column) {
                    column.group_ratios = ratios.clone();
                }
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::GroupRatios(ratios.clone())),
                    inverse: Some(DockCommand::ResizeGroups { column: *column, ratios: current }),
                    persistent: true,
                })
            }
            DockCommand::ResizeColumn { column, width } => {
                if !width.is_finite() || *width <= 0.0 {
                    return Err(CommandError::InvalidValue { field: "width", reason: "width must be finite and positive" });
                }
                let current = match self.nodes.get(*column) {
                    Some(Node::Column(column)) => column.clone(),
                    Some(_) => return Err(CommandError::InvalidTarget { reason: "ResizeColumn target must be a Column node" }),
                    None => return Err(CommandError::MissingNode { node: *column }),
                };
                let old_width = current.width;
                let min_width = self.minimum_column_width(*column, metrics)?;
                let actual_width = match current.mode {
                    ColumnMode::IconsOnly => metrics.rail_width,
                    ColumnMode::Expanded => width.max(min_width),
                };
                if !actual_width.is_finite() || actual_width <= 0.0 {
                    return Err(CommandError::InvalidValue { field: "width", reason: "effective width is not finite and positive" });
                }
                if let Some(Node::Column(column)) = self.nodes.get_mut(*column) {
                    column.width = actual_width;
                }
                self.sync_main_column_ratios(*column)?;
                Ok(CommandApplication {
                    actual_value: Some(CommandValue::ColumnWidth(actual_width)),
                    inverse: (actual_width != old_width).then_some(DockCommand::ResizeColumn { column: *column, width: old_width }),
                    persistent: true,
                })
            }
            DockCommand::ResizeColumnBoundary { side, boundary_index, delta } => {
                if !delta.is_finite() {
                    return Err(CommandError::InvalidValue { field: "delta", reason: "delta must be finite" });
                }
                let columns = self.main_columns(*side)?;
                if *boundary_index >= columns.len().saturating_sub(1) {
                    return Err(CommandError::InvalidTarget { reason: "column boundary index does not identify adjacent columns" });
                }
                let left_id = columns[*boundary_index];
                let right_id = columns[*boundary_index + 1];
                let left = match self.nodes.get(left_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => return Err(CommandError::InvalidTarget { reason: "main dock contains a non-Column child" }),
                };
                let right = match self.nodes.get(right_id) {
                    Some(Node::Column(column)) => column.clone(),
                    _ => return Err(CommandError::InvalidTarget { reason: "main dock contains a non-Column child" }),
                };
                let left_min = self.minimum_column_width(left_id, metrics)?;
                let right_min = self.minimum_column_width(right_id, metrics)?;
                let lower_delta = left_min - left.width;
                let upper_delta = right.width - right_min;
                if lower_delta > upper_delta {
                    return Err(CommandError::InvalidValue {
                        field: "delta",
                        reason: "adjacent columns cannot be resized within their minimum widths",
                    });
                }
                let actual_delta = if left.mode == ColumnMode::IconsOnly || right.mode == ColumnMode::IconsOnly {
                    0.0
                } else {
                    (*delta).clamp(lower_delta, upper_delta)
                };
                if actual_delta != 0.0 {
                    let new_left = left.width + actual_delta;
                    let new_right = right.width - actual_delta;
                    if !new_left.is_finite() || !new_right.is_finite() {
                        return Err(CommandError::InvalidValue { field: "delta", reason: "resized column width overflowed" });
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
            DockCommand::ResetColumnRatios { side } => {
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
                    Some(_) => Err(CommandError::InvalidTarget { reason: "main dock root is not a horizontal column split" }),
                    None => Err(CommandError::InvalidTarget { reason: "main dock root is missing" }),
                }
            }
            DockCommand::Float { node, bounds } => {
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
                self.dock_floating_group(*group, *target, metrics)?;
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
                                    reason: "horizontal group splits require a main-dock column",
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
                self.insert_group_at_column(column, group, index, edge_ratio)
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
        split_ratio: Option<f32>,
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
                    Some(ratio) if ratio.is_finite() && ratio > 0.0 && ratio < 1.0 => {
                        if index == 0 { vec![ratio, 1.0 - ratio] } else { vec![1.0 - ratio, ratio] }
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
                let ratios = if let Some(ratio) = split_ratio {
                    if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                        return Err(CommandError::InvalidValue { field: "ratio", reason: "split ratio must be between zero and one" });
                    }
                    let mut weights = if ratio_issues(&split.ratios, old_len).is_empty() {
                        split.ratios.clone()
                    } else {
                        equal_ratios(old_len)
                    };
                    let target_index = if index < old_len { index } else { old_len - 1 };
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
        split_ratio: Option<f32>,
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
        let ratios = if let Some(ratio) = split_ratio {
            if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
                return Err(CommandError::InvalidValue { field: "ratio", reason: "split ratio must be between zero and one" });
            }
            let old_len = column.groups.len();
            if old_len == 0 {
                vec![1.0]
            } else {
                let mut weights = if ratio_issues(&column.group_ratios, old_len).is_empty() {
                    column.group_ratios.clone()
                } else {
                    equal_ratios(old_len)
                };
                let target_index = if index < old_len { index } else { old_len - 1 };
                let target_weight = weights[target_index];
                weights[target_index] = target_weight * (1.0 - ratio);
                weights.insert(index, target_weight * ratio);
                normalized_ratios(weights.into_iter().map(Some).collect())
            }
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
                        self.insert_group_at_column(column_id, new_group, index, Some(split_ratio))
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
                        self.insert_existing_column_at(side, insert_at, column, Some(split_ratio))
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
                                    let target_index = split.children.iter().position(|id| *id == target_group).unwrap_or(0);
                                    let insert_at = if zone == DropZone::Top { target_index } else { target_index + 1 };
                                    let Some(Node::Split(split)) = self.nodes.get_mut(parent) else {
                                        return Err(CommandError::InvalidTarget { reason: "floating stack changed unexpectedly" });
                                    };
                                    split.children.insert(insert_at, new_group);
                                    split.ratios = equal_ratios(split.children.len());
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
                    for (offset, child) in tabs.children.drain(..).enumerate() {
                        target_tabs.children.insert(insert_at + offset, child);
                    }
                    target_tabs.active_index = insert_at;
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
            canvas,
            profile,
            version,
            chrome,
            flyout: None,
        }
    }
}

#[derive(Clone, Debug)]
struct CommandApplication {
    actual_value: Option<CommandValue>,
    inverse: Option<DockCommand>,
    persistent: bool,
}

impl CommandApplication {
    fn persistent() -> Self {
        Self { actual_value: None, inverse: None, persistent: true }
    }

    fn transient() -> Self {
        Self { actual_value: None, inverse: None, persistent: false }
    }
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
                    target: DropTarget::MainColumn {
                        side: DockSide::Right,
                        index: metrics.max_columns,
                    },
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
    fn apply_move_group_reorders_with_an_inverse_that_restores_the_original_order() {
        let metrics = DockMetrics::default();
        let mut layout = initial_layout(Profile::Generic);
        let column_id = layout.main_area(DockSide::Right).unwrap().root;
        let (first_group, second_group) = match layout.node(column_id).unwrap() {
            Node::Column(column) => (column.groups[0], column.groups[1]),
            _ => panic!("initial main root must be a Column"),
        };

        let moved = layout
            .apply(
                DockCommand::MoveGroup {
                    group: first_group,
                    column: column_id,
                    index: 2,
                },
                &metrics,
            )
            .unwrap();
        let groups_after_move = match layout.node(column_id).unwrap() {
            Node::Column(column) => column.groups.clone(),
            _ => panic!("main root must remain a Column"),
        };
        assert_eq!(groups_after_move, vec![second_group, first_group]);
        assert_eq!(layout.version(), 1);

        let inverse = moved.inverse.expect("a same-column move has a safe inverse");
        layout.apply(inverse, &metrics).unwrap();
        let groups_after_inverse = match layout.node(column_id).unwrap() {
            Node::Column(column) => column.groups.clone(),
            _ => panic!("main root must remain a Column"),
        };
        assert_eq!(groups_after_inverse, vec![first_group, second_group]);
        assert_eq!(layout.version(), 2);

        let moved_backwards = layout
            .apply(
                DockCommand::MoveGroup {
                    group: second_group,
                    column: column_id,
                    index: 0,
                },
                &metrics,
            )
            .unwrap();
        assert!(matches!(
            layout.node(column_id),
            Some(Node::Column(column)) if column.groups == vec![second_group, first_group]
        ));
        layout.apply(moved_backwards.inverse.expect("backward move has a safe inverse"), &metrics).unwrap();
        assert!(matches!(
            layout.node(column_id),
            Some(Node::Column(column)) if column.groups == vec![first_group, second_group]
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

}
