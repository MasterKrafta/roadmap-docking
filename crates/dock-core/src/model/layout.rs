use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::geometry::{DockMetrics, Size};
use crate::ids::{FloatingGroupId, NodeId, PanelId, PanelKey};

use super::{
    Axis, CanvasState, Column, ColumnMode, DockArea, DockSide, FloatingGroup, Node,
    PanelConstraints, PanelInstance, Profile, Split, Tabs, ValidationError,
};

const PIXEL_TOLERANCE: f32 = 1.0e-3;
const RATIO_TOLERANCE: f32 = 1.0e-4;

/// Complete dock layout model, independent of GPUI, windows, and panel content.
///
/// Equality is intentionally not derived because SlotMap does not implement PartialEq;
/// compare the public model projections or focused values in tests instead.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panels: SlotMap<PanelId, PanelInstance>,
    dock_areas: Vec<DockArea>,
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
    /// Session-only fallback positions used when reopening recently closed panels.
    #[cfg_attr(feature = "serde", serde(skip))]
    last_panel_locations: Vec<(PanelKey, DockSide, usize, usize)>,
}

impl Layout {
    pub fn new(profile: Profile, canvas: CanvasState) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panels: SlotMap::with_key(),
            dock_areas: Vec::new(),
            floating_groups: SlotMap::with_key(),
            canvas,
            profile,
            version: 0,
            last_panel_locations: Vec::new(),
        }
    }

    /// Create the initial Photoshop composition without depending on GPUI.
    ///
    /// The four built-in panels each have one instance. The Canvas exists as a protected
    /// region in the model; its actual window-relative geometry is assigned by the layout
    /// geometry stage. Panel constraints are provisional until the app registry is added.
    pub fn initial_photoshop(metrics: &DockMetrics) -> Result<Self, &'static str> {
        metrics.validate()?;

        let constraints = PanelConstraints::new(
            Size::new(metrics.min_column_width, metrics.min_group_height),
            Size::new(metrics.default_column_width, metrics.min_group_height),
            Size::new(f32::MAX, f32::MAX),
            true,
        );

        let mut layout = Self::new(Profile::Photoshop, CanvasState::default());
        let layers = layout.insert_initial_panel("layers", constraints)?;
        let color = layout.insert_initial_panel("color", constraints)?;
        let properties = layout.insert_initial_panel("properties", constraints)?;
        let history = layout.insert_initial_panel("history", constraints)?;

        let layers_group = layout.insert_node(Node::Tabs(Tabs {
            panels: vec![layers, color],
            active_index: 0,
        }));
        let properties_group = layout.insert_node(Node::Tabs(Tabs {
            panels: vec![properties, history],
            active_index: 0,
        }));

        layout.push_dock_area(DockArea {
            side: DockSide::Right,
            columns: vec![Column {
                width: metrics.default_column_width,
                mode: ColumnMode::Expanded,
                groups: vec![layers_group, properties_group],
                group_ratios: vec![0.5, 0.5],
            }],
        });

        Ok(layout)
    }

    fn insert_initial_panel(
        &mut self,
        key: &str,
        constraints: PanelConstraints,
    ) -> Result<PanelId, &'static str> {
        let key = PanelKey::new(key)?;
        Ok(self.insert_panel(PanelInstance::new(key, constraints)))
    }

    pub fn nodes(&self) -> &SlotMap<NodeId, Node> {
        &self.nodes
    }

    pub fn panels(&self) -> &SlotMap<PanelId, PanelInstance> {
        &self.panels
    }

    pub fn dock_areas(&self) -> &[DockArea] {
        &self.dock_areas
    }

    pub fn floating_groups(&self) -> &SlotMap<FloatingGroupId, FloatingGroup> {
        &self.floating_groups
    }

    pub const fn canvas(&self) -> &CanvasState {
        &self.canvas
    }

    pub const fn profile(&self) -> Profile {
        self.profile
    }

    pub const fn version(&self) -> u64 {
        self.version
    }

    /// Apply one docking command atomically.
    ///
    /// The command mutates a candidate clone only. Normalization and validation must
    /// both succeed before the candidate replaces this layout, so rejected commands
    /// preserve the full original state and version.
    pub fn apply(
        &mut self,
        command: crate::command::DockCommand,
        metrics: &DockMetrics,
    ) -> Result<crate::command::CommandOutcome, crate::command::CommandError> {
        use crate::command::{CommandError, CommandOutcome};

        tracing::debug!(version = self.version, command = ?command, "applying dock command");

        let mut candidate = self.clone();
        let mut actual_value = None;
        let mut changed_nodes =
            match candidate.apply_command_inner(command, metrics, &mut actual_value) {
                Ok(changed) => changed,
                Err(error) => {
                    tracing::warn!(version = self.version, error = %error, "dock command rejected");
                    return Err(error);
                }
            };

        let normalized = candidate.normalize(metrics);
        if let Err(errors) = normalized {
            let error = CommandError::InvalidLayout {
                reasons: errors.into_iter().map(|error| error.to_string()).collect(),
            };
            tracing::warn!(version = self.version, error = %error, "dock command rejected");
            return Err(error);
        }
        if let Err(errors) = candidate.validate(metrics) {
            let error = CommandError::InvalidLayout {
                reasons: errors.into_iter().map(|error| error.to_string()).collect(),
            };
            tracing::warn!(version = self.version, error = %error, "dock command rejected");
            return Err(error);
        }

        let Some(next_version) = self.version.checked_add(1) else {
            let error = CommandError::VersionOverflow;
            tracing::warn!(version = self.version, error = %error, "dock command rejected");
            return Err(error);
        };
        candidate.version = next_version;

        let mut seen = HashSet::new();
        changed_nodes.retain(|node_id| seen.insert(*node_id));
        let outcome =
            CommandOutcome::new(changed_nodes, candidate.version).with_actual_value(actual_value);
        *self = candidate;
        Ok(outcome)
    }

    fn apply_command_inner(
        &mut self,
        command: crate::command::DockCommand,
        metrics: &DockMetrics,
        actual_value: &mut Option<f32>,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, DockCommand};

        match command {
            DockCommand::ActivateTab { group, index } => {
                let Some(node) = self.nodes.get_mut(group) else {
                    return Err(CommandError::UnknownGroup(group));
                };
                let Node::Tabs(tabs) = node else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                if index >= tabs.panels.len() {
                    return Err(CommandError::InvalidTabIndex { group, index });
                }
                tabs.active_index = index;
                Ok(vec![group])
            }
            DockCommand::ActivateFloatingTab { floating, index } => {
                let Some(group) = self.floating_groups.get_mut(floating) else {
                    return Err(CommandError::UnknownFloatingGroup(floating));
                };
                if index >= group.panels.len() {
                    return Err(CommandError::InvalidCommandParameter {
                        parameter: "floating tab index".to_owned(),
                        reason: format!(
                            "index {index} must be smaller than floating group tab count {}",
                            group.panels.len()
                        ),
                    });
                }
                group.active_index = index;
                Ok(Vec::new())
            }
            DockCommand::ReorderTab { group, from, to } => {
                let Some(node) = self.nodes.get_mut(group) else {
                    return Err(CommandError::UnknownGroup(group));
                };
                let Node::Tabs(tabs) = node else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                let len = tabs.panels.len();
                if from >= len {
                    return Err(CommandError::InvalidTabIndex { group, index: from });
                }
                if to >= len {
                    return Err(CommandError::InvalidTabIndex { group, index: to });
                }

                let active_panel = tabs.panels.get(tabs.active_index).copied();
                let moved_panel = tabs.panels.remove(from);
                tabs.panels.insert(to, moved_panel);
                if let Some(active_panel) = active_panel {
                    if let Some(active_index) =
                        tabs.panels.iter().position(|panel| *panel == active_panel)
                    {
                        tabs.active_index = active_index;
                    }
                }
                Ok(vec![group])
            }
            DockCommand::MovePanel { panel, target } => match target {
                crate::command::DropTarget::FloatingGroup { floating, zone } => {
                    self.move_panel_to_floating_group(panel, floating, zone, metrics)
                }
                target @ crate::command::DropTarget::Group {
                    zone: crate::command::DropZone::Top | crate::command::DropZone::Bottom,
                    ..
                }
                | target @ crate::command::DropTarget::Column { .. }
                | target @ crate::command::DropTarget::DockEdge { .. } => {
                    self.move_panel_to_dock_side(panel, target, metrics)
                }
                target => self.move_panel_to_existing_group(panel, target),
            },
            DockCommand::Float { source, rect } => self.float_source(source, rect, metrics),
            DockCommand::MoveFloating { floating, rect } => {
                self.move_floating(floating, rect, metrics)
            }
            DockCommand::ResizeFloating { floating, rect } => {
                self.resize_floating(floating, rect, metrics)
            }
            DockCommand::RaiseFloating { floating } => self.raise_floating(floating),
            DockCommand::DockFloating { floating, target } => {
                self.dock_floating(floating, target, metrics)
            }
            DockCommand::SplitGroup { panel, group, side } => {
                self.split_group(panel, group, side, metrics)
            }
            DockCommand::ResizeGroups {
                column,
                boundary,
                delta,
            } => self.resize_groups(column, boundary, delta, metrics, actual_value),
            DockCommand::ResizeColumn { column, width } => {
                self.resize_column(column, width, metrics, actual_value)
            }
            DockCommand::ResizeColumnBoundary {
                first,
                second,
                delta,
            } => self.resize_column_boundary(first, second, delta, metrics, actual_value),
            DockCommand::ClosePanel { panel } => self.close_panel(panel),
            DockCommand::CloseGroup { group } => self.close_group(group),
            DockCommand::OpenPanel {
                key,
                constraints,
                target,
            } => self.open_panel(key, constraints, target, metrics),
            other => {
                let name = match other {
                    DockCommand::SplitGroup { .. } => "SplitGroup",
                    DockCommand::ResizeGroups { .. } => "ResizeGroups",
                    DockCommand::ResizeColumn { .. } => "ResizeColumn",
                    DockCommand::ResizeColumnBoundary { .. } => "ResizeColumnBoundary",
                    DockCommand::ResetColumnRatios { .. } => "ResetColumnRatios",
                    DockCommand::Float { .. } => "Float",
                    DockCommand::MoveFloating { .. } => "MoveFloating",
                    DockCommand::ResizeFloating { .. } => "ResizeFloating",
                    DockCommand::RaiseFloating { .. } => "RaiseFloating",
                    DockCommand::DockFloating { .. } => "DockFloating",
                    DockCommand::OpenPanel { .. } => "OpenPanel",
                    DockCommand::ClosePanel { .. } => "ClosePanel",
                    DockCommand::CloseGroup { .. } => "CloseGroup",
                    DockCommand::ActivateFloatingTab { .. } => "ActivateFloatingTab",
                    DockCommand::ActivateTab { .. } => "ActivateTab",
                    DockCommand::ReorderTab { .. } => "ReorderTab",
                    DockCommand::MovePanel { .. } => "MovePanel",
                };
                Err(CommandError::UnsupportedCommand(name.to_owned()))
            }
        }
    }

    fn split_group(
        &mut self,
        panel: PanelId,
        group: NodeId,
        side: crate::command::SplitSide,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, SplitSide};

        if self.profile != Profile::Photoshop {
            return Err(CommandError::UnsupportedForProfile {
                command: "SplitGroup".to_owned(),
                profile: self.profile,
            });
        }
        if !self.panels.contains_key(panel) {
            return Err(CommandError::UnknownPanel(panel));
        }

        let location = self
            .dock_areas
            .iter()
            .enumerate()
            .find_map(|(area_index, area)| {
                area.columns
                    .iter()
                    .enumerate()
                    .find_map(|(column_index, column)| {
                        column
                            .groups
                            .iter()
                            .position(|candidate| *candidate == group)
                            .map(|group_index| (area_index, column_index, group_index))
                    })
            })
            .ok_or(CommandError::UnknownGroup(group))?;

        let (area_index, column_index, group_index) = location;
        let (panel_index, source_is_empty_after_move, original_ratio) = {
            let Some(Node::Tabs(tabs)) = self.nodes.get(group) else {
                return if self.nodes.contains_key(group) {
                    Err(CommandError::GroupNotTabs(group))
                } else {
                    Err(CommandError::UnknownGroup(group))
                };
            };
            let Some(panel_index) = tabs.panels.iter().position(|candidate| *candidate == panel)
            else {
                return Err(CommandError::PanelNotInGroup { panel, group });
            };
            let ratio = self.dock_areas[area_index].columns[column_index]
                .group_ratios
                .get(group_index)
                .copied()
                .ok_or_else(|| CommandError::InvalidCommandParameter {
                    parameter: "group_ratios".to_owned(),
                    reason: "target group has no matching height ratio".to_owned(),
                })?;
            if !ratio.is_finite() || ratio < 0.0 {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "group_ratios".to_owned(),
                    reason: "target group has an invalid height ratio".to_owned(),
                });
            }
            (panel_index, tabs.panels.len() == 1, ratio)
        };

        let constraints = self
            .panels
            .get(panel)
            .ok_or(CommandError::UnknownPanel(panel))?
            .constraints();
        if !panel_constraints_are_valid(constraints) {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "PanelConstraints".to_owned(),
                reason: "the panel's minimum, preferred, and maximum sizes are inconsistent"
                    .to_owned(),
            });
        }
        let min_size = constraints.min_size();

        match side {
            SplitSide::Top | SplitSide::Bottom => {
                // The model stores proportional heights, not a window height. We can
                // split the group's current share atomically here; R4 geometry will
                // enforce the physical minimum allocations against the actual viewport.
                let insertion_index = if side == SplitSide::Top {
                    group_index
                } else {
                    group_index + 1
                };
                let new_group = self.insert_node(Node::Tabs(Tabs {
                    panels: vec![panel],
                    active_index: 0,
                }));

                let Some(Node::Tabs(source_tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                Self::remove_panel_from_tabs(source_tabs, panel_index);

                let column = &mut self.dock_areas[area_index].columns[column_index];
                if source_is_empty_after_move {
                    // The empty source group will be removed by normalization. Give the
                    // new group the source group's full share; normalization removes the
                    // old group's paired ratio without shrinking the remaining siblings.
                    column.groups.insert(insertion_index, new_group);
                    column.group_ratios.insert(insertion_index, original_ratio);
                } else {
                    let half = original_ratio * 0.5;
                    column.group_ratios[group_index] = half;
                    column.groups.insert(insertion_index, new_group);
                    column.group_ratios.insert(insertion_index, half);
                }

                self.ensure_column_fits_min_height(area_index, column_index, metrics)?;
                Ok(vec![group, new_group])
            }
            SplitSide::Left | SplitSide::Right => {
                let current_columns: usize =
                    self.dock_areas.iter().map(|area| area.columns.len()).sum();
                if current_columns >= metrics.max_columns {
                    return Err(CommandError::ColumnLimitReached {
                        maximum: metrics.max_columns,
                    });
                }

                // New columns must be able to contain the dragged panel at its minimum
                // width. Width remains independent from the height ratios of the column.
                let width = metrics
                    .default_column_width
                    .max(metrics.min_column_width)
                    .max(min_size.width);
                if !width.is_finite() || width < metrics.min_column_width {
                    return Err(CommandError::InvalidCommandParameter {
                        parameter: "column width".to_owned(),
                        reason: "new column cannot satisfy its minimum width".to_owned(),
                    });
                }

                let new_group = self.insert_node(Node::Tabs(Tabs {
                    panels: vec![panel],
                    active_index: 0,
                }));
                let Some(Node::Tabs(source_tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                Self::remove_panel_from_tabs(source_tabs, panel_index);

                let new_column = Column {
                    width,
                    mode: ColumnMode::Expanded,
                    groups: vec![new_group],
                    group_ratios: vec![1.0],
                };
                let insert_at = if side == SplitSide::Left {
                    column_index
                } else {
                    column_index + 1
                };
                self.dock_areas[area_index]
                    .columns
                    .insert(insert_at, new_column);
                let source_column_after_insert =
                    column_index + usize::from(side == SplitSide::Left);
                self.ensure_column_fits_min_height(
                    area_index,
                    source_column_after_insert,
                    metrics,
                )?;
                self.ensure_column_fits_min_height(area_index, insert_at, metrics)?;
                Ok(vec![group, new_group])
            }
        }
    }

    fn validate_floating_rect(
        &self,
        rect: crate::geometry::Rect,
        panels: &[PanelId],
        metrics: &DockMetrics,
    ) -> Result<(), crate::command::CommandError> {
        use crate::command::CommandError;

        if !rect.origin.x.is_finite()
            || !rect.origin.y.is_finite()
            || !rect.size.width.is_finite()
            || !rect.size.height.is_finite()
            || rect.size.width <= 0.0
            || rect.size.height <= 0.0
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "floating rect".to_owned(),
                reason: "origin must be finite and size must be finite and positive".to_owned(),
            });
        }

        let mut minimum_width = metrics.min_column_width;
        let mut minimum_height = metrics.min_group_height;
        for panel_id in panels {
            let Some(panel) = self.panels.get(*panel_id) else {
                return Err(CommandError::UnknownPanel(*panel_id));
            };
            let constraints = panel.constraints();
            if !panel_constraints_are_valid(constraints) {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "PanelConstraints".to_owned(),
                    reason: "the panel's minimum, preferred, and maximum sizes are inconsistent"
                        .to_owned(),
                });
            }
            minimum_width = minimum_width.max(constraints.min_size().width);
            minimum_height = minimum_height.max(constraints.min_size().height);
        }
        if !minimum_width.is_finite()
            || !minimum_height.is_finite()
            || rect.size.width + PIXEL_TOLERANCE < minimum_width
            || rect.size.height + PIXEL_TOLERANCE < minimum_height
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "floating rect".to_owned(),
                reason: format!(
                    "floating bounds must be at least {minimum_width:.3} x {minimum_height:.3} logical px"
                ),
            });
        }
        Ok(())
    }

    fn next_floating_z_order(&mut self) -> u64 {
        let maximum = self
            .floating_groups
            .values()
            .map(|group| group.z_order)
            .max();
        if let Some(next) = maximum.and_then(|value| value.checked_add(1)) {
            return next;
        }

        // Compact an exhausted z-order range without changing the relative order.
        let mut ordered: Vec<(FloatingGroupId, u64)> = self
            .floating_groups
            .iter()
            .map(|(id, group)| (id, group.z_order))
            .collect();
        ordered.sort_by_key(|(_, z_order)| *z_order);
        for (rank, (id, _)) in ordered.iter().enumerate() {
            if let Some(group) = self.floating_groups.get_mut(*id) {
                group.z_order = rank as u64;
            }
        }
        ordered.len() as u64
    }

    fn float_source(
        &mut self,
        source: crate::command::FloatSource,
        rect: crate::geometry::Rect,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, FloatSource};

        let (panels, active_index, changed_nodes) = match source {
            FloatSource::Panel(panel) => {
                if !self.panels.contains_key(panel) {
                    return Err(CommandError::UnknownPanel(panel));
                }
                let Some((group, panel_index)) = self.find_dock_group_for_panel(panel) else {
                    return Err(CommandError::PanelNotInDockGroup(panel));
                };
                if self.find_dock_group_location(group).is_none() {
                    return Err(CommandError::PanelNotInDockGroup(panel));
                }
                if !matches!(self.nodes.get(group), Some(Node::Tabs(_))) {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                }
                self.validate_floating_rect(rect, &[panel], metrics)?;
                // Keep the prior dock position so closing this floating panel can restore it.
                self.remember_panel_location(panel, group);
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                Self::remove_panel_from_tabs(tabs, panel_index);
                (vec![panel], 0, vec![group])
            }
            FloatSource::Group(group) => {
                let Some((side, column_index, group_index)) = self.find_dock_group_location(group)
                else {
                    return Err(CommandError::UnknownGroup(group));
                };
                let area_index = self
                    .dock_areas
                    .iter()
                    .position(|area| {
                        area.side == side
                            && area
                                .columns
                                .get(column_index)
                                .and_then(|column| column.groups.get(group_index))
                                == Some(&group)
                    })
                    .ok_or(CommandError::UnknownGroup(group))?;
                let (panels, active_index) = match self.nodes.get(group) {
                    Some(Node::Tabs(tabs)) if !tabs.panels.is_empty() => {
                        (tabs.panels.clone(), tabs.active_index)
                    }
                    Some(Node::Tabs(_)) => {
                        return Err(CommandError::InvalidCommandParameter {
                            parameter: "floating source group".to_owned(),
                            reason: "cannot float an empty group".to_owned(),
                        });
                    }
                    Some(Node::Split(_)) => return Err(CommandError::GroupNotTabs(group)),
                    None => return Err(CommandError::UnknownGroup(group)),
                };
                self.validate_floating_rect(rect, &panels, metrics)?;
                // Preserve a per-panel fallback to this group's former dock location.
                // If the floating group is later closed, each panel can return here.
                for panel in &panels {
                    self.remember_panel_location(*panel, group);
                }
                let column = self
                    .dock_areas
                    .get_mut(area_index)
                    .and_then(|area| area.columns.get_mut(column_index))
                    .ok_or(CommandError::UnknownGroup(group))?;
                if column.groups.get(group_index) != Some(&group)
                    || column.group_ratios.len() != column.groups.len()
                {
                    return Err(CommandError::InvalidCommandParameter {
                        parameter: "floating source group".to_owned(),
                        reason: "source column references or height ratios are inconsistent"
                            .to_owned(),
                    });
                }
                column.groups.remove(group_index);
                column.group_ratios.remove(group_index);
                normalize_ratios(&mut column.group_ratios, column.groups.len());
                (panels, active_index, vec![group])
            }
        };

        let z_order = self.next_floating_z_order();
        self.insert_floating_group(FloatingGroup {
            panels,
            active_index,
            bounds: rect,
            z_order,
        });
        Ok(changed_nodes)
    }

    fn move_floating(
        &mut self,
        floating_id: FloatingGroupId,
        rect: crate::geometry::Rect,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        let Some(group) = self.floating_groups.get(floating_id) else {
            return Err(CommandError::UnknownFloatingGroup(floating_id));
        };
        self.validate_floating_rect(rect, &group.panels, metrics)?;
        let Some(group) = self.floating_groups.get_mut(floating_id) else {
            return Err(CommandError::UnknownFloatingGroup(floating_id));
        };
        group.bounds.origin = rect.origin;
        Ok(Vec::new())
    }

    fn resize_floating(
        &mut self,
        floating_id: FloatingGroupId,
        rect: crate::geometry::Rect,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        let Some(group) = self.floating_groups.get(floating_id) else {
            return Err(CommandError::UnknownFloatingGroup(floating_id));
        };
        self.validate_floating_rect(rect, &group.panels, metrics)?;
        let Some(group) = self.floating_groups.get_mut(floating_id) else {
            return Err(CommandError::UnknownFloatingGroup(floating_id));
        };
        // Resize commands carry the complete rectangle: left/top edge drags
        // must move the origin as the opposite edge remains anchored.
        group.bounds = rect;
        Ok(Vec::new())
    }

    fn raise_floating(
        &mut self,
        floating_id: FloatingGroupId,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        if !self.floating_groups.contains_key(floating_id) {
            return Err(CommandError::UnknownFloatingGroup(floating_id));
        }
        let z_order = self.next_floating_z_order();
        let Some(group) = self.floating_groups.get_mut(floating_id) else {
            return Err(CommandError::UnknownFloatingGroup(floating_id));
        };
        group.z_order = z_order;
        Ok(Vec::new())
    }

    fn move_panel_to_floating_group(
        &mut self,
        panel: PanelId,
        target_id: FloatingGroupId,
        zone: crate::command::DropZone,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, DropZone};

        if zone != DropZone::Center {
            return Err(CommandError::InvalidDropTarget(
                "MovePanel into a floating group only supports the Center zone".to_owned(),
            ));
        }
        if !self.panels.contains_key(panel) {
            return Err(CommandError::UnknownPanel(panel));
        }
        let Some(target_group) = self.floating_groups.get(target_id) else {
            return Err(CommandError::UnknownFloatingGroup(target_id));
        };
        if let Some(index) = target_group
            .panels
            .iter()
            .position(|candidate| *candidate == panel)
        {
            if let Some(target_group) = self.floating_groups.get_mut(target_id) {
                target_group.active_index = index;
            }
            return Ok(Vec::new());
        }
        let target_bounds = target_group.bounds;
        let mut prospective_panels = target_group.panels.clone();
        prospective_panels.push(panel);
        self.validate_floating_rect(target_bounds, &prospective_panels, metrics)?;

        let dock_source = self
            .find_dock_group_for_panel(panel)
            .filter(|(group, _)| self.find_dock_group_location(*group).is_some());
        let floating_source = self.floating_groups.iter().find_map(|(id, group)| {
            group
                .panels
                .iter()
                .position(|candidate| *candidate == panel)
                .map(|index| (id, index))
        });
        if dock_source.is_none() && floating_source.is_none() {
            return Err(CommandError::PanelNotPlaced(panel));
        }

        let mut changed_nodes = Vec::new();
        if let Some((source_group, source_index)) = dock_source {
            let Some(Node::Tabs(tabs)) = self.nodes.get_mut(source_group) else {
                return Err(CommandError::GroupNotTabs(source_group));
            };
            Self::remove_panel_from_tabs(tabs, source_index);
            changed_nodes.push(source_group);
        } else if let Some((source_id, source_index)) = floating_source {
            if source_id != target_id {
                if let Some(source) = self.floating_groups.get_mut(source_id) {
                    source.panels.remove(source_index);
                    if source.panels.is_empty() {
                        self.floating_groups.remove(source_id);
                    } else if source_index < source.active_index {
                        source.active_index -= 1;
                    } else if source_index == source.active_index {
                        source.active_index = source.active_index.min(source.panels.len() - 1);
                    }
                }
            }
        }

        let Some(target_group) = self.floating_groups.get_mut(target_id) else {
            return Err(CommandError::UnknownFloatingGroup(target_id));
        };
        let insert_index = target_group.panels.len();
        target_group.panels.push(panel);
        target_group.active_index = insert_index;
        Ok(changed_nodes)
    }

    fn dock_floating(
        &mut self,
        floating_id: FloatingGroupId,
        target: crate::command::DropTarget,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, DropTarget, DropZone};
        use crate::model::DockSide;

        let Some(source) = self.floating_groups.get(floating_id).cloned() else {
            return Err(CommandError::UnknownFloatingGroup(floating_id));
        };
        self.validate_floating_rect(source.bounds, &source.panels, metrics)?;
        if source.panels.is_empty() {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "floating group".to_owned(),
                reason: "cannot dock an empty floating group".to_owned(),
            });
        }

        let mut changed_nodes = Vec::new();
        match target {
            DropTarget::Group {
                group,
                zone: DropZone::Center,
            } => {
                let Some(Node::Tabs(tabs)) = self.nodes.get(group) else {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                };
                self.ensure_group_accepts_panels_width(group, &source.panels, metrics)?;
                let insert_at = tabs.panels.len();
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                tabs.panels.extend(source.panels.iter().copied());
                tabs.active_index = insert_at + source.active_index;
                changed_nodes.push(group);
            }
            DropTarget::TabInsertion { group, index } => {
                let Some(Node::Tabs(tabs)) = self.nodes.get(group) else {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                };
                self.ensure_group_accepts_panels_width(group, &source.panels, metrics)?;
                if index > tabs.panels.len() {
                    return Err(CommandError::InvalidTabIndex { group, index });
                }
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                for (offset, panel) in source.panels.iter().copied().enumerate() {
                    tabs.panels.insert(index + offset, panel);
                }
                tabs.active_index = index + source.active_index;
                changed_nodes.push(group);
            }
            DropTarget::Group { group, zone } => {
                let Some((side, column_index, group_index)) = self.find_dock_group_location(group)
                else {
                    return Err(CommandError::UnknownGroup(group));
                };
                if !matches!(self.nodes.get(group), Some(Node::Tabs(_))) {
                    return Err(CommandError::GroupNotTabs(group));
                }
                if zone == DropZone::Top || zone == DropZone::Bottom {
                    self.ensure_group_accepts_panels_width(group, &source.panels, metrics)?;
                    let area_index = self
                        .dock_areas
                        .iter()
                        .position(|area| {
                            area.side == side
                                && area
                                    .columns
                                    .get(column_index)
                                    .and_then(|column| column.groups.get(group_index))
                                    == Some(&group)
                        })
                        .ok_or(CommandError::UnknownGroup(group))?;
                    let new_group = self.insert_node(Node::Tabs(Tabs {
                        panels: source.panels.clone(),
                        active_index: source.active_index,
                    }));
                    let column = &mut self.dock_areas[area_index].columns[column_index];
                    if column.group_ratios.len() != column.groups.len() {
                        return Err(CommandError::InvalidCommandParameter {
                            parameter: "group_ratios".to_owned(),
                            reason: "there must be one ratio for every group".to_owned(),
                        });
                    }
                    let ratio = column.group_ratios[group_index];
                    if !ratio.is_finite() || ratio < 0.0 {
                        return Err(CommandError::InvalidCommandParameter {
                            parameter: "group_ratios".to_owned(),
                            reason: "target group ratio must be finite and non-negative".to_owned(),
                        });
                    }
                    let half = ratio * 0.5;
                    let insert_at = if zone == DropZone::Top {
                        group_index
                    } else {
                        group_index + 1
                    };
                    column.group_ratios[group_index] = half;
                    column.groups.insert(insert_at, new_group);
                    column.group_ratios.insert(insert_at, half);
                    self.ensure_column_fits_min_height(area_index, column_index, metrics)?;
                    changed_nodes.extend([group, new_group]);
                } else if zone == DropZone::Left || zone == DropZone::Right {
                    let area_index = self
                        .dock_areas
                        .iter()
                        .position(|area| area.side == side)
                        .ok_or(CommandError::UnknownGroup(group))?;
                    self.ensure_can_add_column(metrics)?;
                    let width = self.minimum_floating_dock_width(&source.panels, metrics)?;
                    let new_group = self.insert_node(Node::Tabs(Tabs {
                        panels: source.panels.clone(),
                        active_index: source.active_index,
                    }));
                    let column = Column {
                        width,
                        mode: ColumnMode::Expanded,
                        groups: vec![new_group],
                        group_ratios: vec![1.0],
                    };
                    let insert_at = column_index + usize::from(zone == DropZone::Right);
                    self.dock_areas[area_index]
                        .columns
                        .insert(insert_at, column);
                    self.ensure_column_fits_min_height(area_index, insert_at, metrics)?;
                    changed_nodes.extend([group, new_group]);
                } else {
                    return Err(CommandError::InvalidDropTarget(
                        "unsupported group zone for DockFloating".to_owned(),
                    ));
                }
            }
            DropTarget::Column { column, zone } => {
                let (area_index, column_index) = self.resolve_column_ref(column)?;
                if zone == DropZone::Left || zone == DropZone::Right {
                    self.ensure_can_add_column(metrics)?;
                    let width = self.minimum_floating_dock_width(&source.panels, metrics)?;
                    let new_group = self.insert_node(Node::Tabs(Tabs {
                        panels: source.panels.clone(),
                        active_index: source.active_index,
                    }));
                    let new_column = Column {
                        width,
                        mode: ColumnMode::Expanded,
                        groups: vec![new_group],
                        group_ratios: vec![1.0],
                    };
                    let insert_at = column_index + usize::from(zone == DropZone::Right);
                    self.dock_areas[area_index]
                        .columns
                        .insert(insert_at, new_column);
                    self.ensure_column_fits_min_height(area_index, insert_at, metrics)?;
                    changed_nodes.push(new_group);
                } else {
                    self.ensure_panels_fit_column(
                        &source.panels,
                        area_index,
                        column_index,
                        metrics,
                    )?;
                    let new_group = self.insert_node(Node::Tabs(Tabs {
                        panels: source.panels.clone(),
                        active_index: source.active_index,
                    }));
                    let column = &mut self.dock_areas[area_index].columns[column_index];
                    let insert_at = if zone == DropZone::Top {
                        0
                    } else {
                        column.groups.len()
                    };
                    column.groups.insert(insert_at, new_group);
                    column.group_ratios.insert(insert_at, 1.0);
                    normalize_ratios(&mut column.group_ratios, column.groups.len());
                    self.ensure_column_fits_min_height(area_index, column_index, metrics)?;
                    changed_nodes.push(new_group);
                }
            }
            DropTarget::DockEdge { side } => {
                self.ensure_can_add_column(metrics)?;
                let width = self.minimum_floating_dock_width(&source.panels, metrics)?;
                let new_group = self.insert_node(Node::Tabs(Tabs {
                    panels: source.panels.clone(),
                    active_index: source.active_index,
                }));
                let new_column = Column {
                    width,
                    mode: ColumnMode::Expanded,
                    groups: vec![new_group],
                    group_ratios: vec![1.0],
                };
                let insert_at = self
                    .dock_areas
                    .iter()
                    .position(|area| area.side == side)
                    .map(|area_index| {
                        if side == DockSide::Left {
                            0
                        } else {
                            self.dock_areas[area_index].columns.len()
                        }
                    });
                if let Some(area_index) = self.dock_areas.iter().position(|area| area.side == side)
                {
                    let index = insert_at.unwrap_or(0);
                    self.dock_areas[area_index]
                        .columns
                        .insert(index, new_column);
                    self.ensure_column_fits_min_height(area_index, index, metrics)?;
                } else {
                    self.dock_areas.push(DockArea {
                        side,
                        columns: vec![new_column],
                    });
                    let area_index = self.dock_areas.len() - 1;
                    self.ensure_column_fits_min_height(area_index, 0, metrics)?;
                }
                changed_nodes.push(new_group);
            }
            DropTarget::FloatingGroup {
                floating: target_id,
                zone,
            } => {
                if target_id == floating_id {
                    return Err(CommandError::InvalidDropTarget(
                        "a floating group cannot be docked into itself".to_owned(),
                    ));
                }
                if zone != DropZone::Center {
                    return Err(CommandError::InvalidDropTarget(
                        "merging floating groups only supports the Center zone".to_owned(),
                    ));
                }
                let Some(target_group) = self.floating_groups.get(target_id) else {
                    return Err(CommandError::UnknownFloatingGroup(target_id));
                };
                let mut prospective_panels = target_group.panels.clone();
                prospective_panels.extend(source.panels.iter().copied());
                self.validate_floating_rect(target_group.bounds, &prospective_panels, metrics)?;
                let Some(target_group) = self.floating_groups.get_mut(target_id) else {
                    return Err(CommandError::UnknownFloatingGroup(target_id));
                };
                let insert_at = target_group.panels.len();
                target_group.panels.extend(source.panels.iter().copied());
                target_group.active_index = insert_at + source.active_index;
            }
            DropTarget::FreeArea { .. } => {
                return Err(CommandError::InvalidDropTarget(
                    "FreeArea creates or positions a floating group; it is not a dock target"
                        .to_owned(),
                ));
            }
        }

        self.floating_groups.remove(floating_id);
        Ok(changed_nodes)
    }

    fn ensure_group_accepts_panels_width(
        &self,
        group_id: NodeId,
        incoming_panels: &[PanelId],
        metrics: &DockMetrics,
    ) -> Result<(), crate::command::CommandError> {
        use crate::command::CommandError;

        let Some(Node::Tabs(tabs)) = self.nodes.get(group_id) else {
            return if self.nodes.contains_key(group_id) {
                Err(CommandError::GroupNotTabs(group_id))
            } else {
                Err(CommandError::UnknownGroup(group_id))
            };
        };
        let Some((area_index, column_index)) =
            self.dock_areas
                .iter()
                .enumerate()
                .find_map(|(area_index, area)| {
                    area.columns
                        .iter()
                        .position(|column| column.groups.contains(&group_id))
                        .map(|column_index| (area_index, column_index))
                })
        else {
            return Err(CommandError::UnknownGroup(group_id));
        };

        let mut all_panels = tabs.panels.clone();
        all_panels.extend(incoming_panels.iter().copied());
        self.ensure_panels_fit_column(&all_panels, area_index, column_index, metrics)
    }

    fn ensure_panels_fit_column(
        &self,
        panels: &[PanelId],
        area_index: usize,
        column_index: usize,
        metrics: &DockMetrics,
    ) -> Result<(), crate::command::CommandError> {
        use crate::command::CommandError;

        let Some(column) = self
            .dock_areas
            .get(area_index)
            .and_then(|area| area.columns.get(column_index))
        else {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "column".to_owned(),
                reason: "target column does not exist".to_owned(),
            });
        };
        let base_minimum = match column.mode {
            ColumnMode::Expanded => metrics.min_column_width,
            ColumnMode::IconsAndLabels | ColumnMode::IconsOnly => metrics.rail_width,
        };
        let mut required_width = base_minimum;
        for panel_id in panels {
            let Some(panel) = self.panels.get(*panel_id) else {
                return Err(CommandError::UnknownPanel(*panel_id));
            };
            if !panel_constraints_are_valid(panel.constraints()) {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "PanelConstraints".to_owned(),
                    reason: "the panel's minimum, preferred, and maximum sizes are inconsistent"
                        .to_owned(),
                });
            }
            required_width = required_width.max(panel.constraints().min_size().width);
        }
        if !column.width.is_finite()
            || column.width <= 0.0
            || !required_width.is_finite()
            || column.width + PIXEL_TOLERANCE < required_width
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "column width".to_owned(),
                reason: format!(
                    "target column must be at least {required_width:.3} logical px wide for the incoming panels"
                ),
            });
        }
        Ok(())
    }

    fn ensure_can_add_column(
        &self,
        metrics: &DockMetrics,
    ) -> Result<(), crate::command::CommandError> {
        use crate::command::CommandError;
        if self.profile == Profile::Photoshop {
            let actual: usize = self.dock_areas.iter().map(|area| area.columns.len()).sum();
            if actual >= metrics.max_columns {
                return Err(CommandError::ColumnLimitReached {
                    maximum: metrics.max_columns,
                });
            }
        }
        Ok(())
    }

    fn minimum_floating_dock_width(
        &self,
        panels: &[PanelId],
        metrics: &DockMetrics,
    ) -> Result<f32, crate::command::CommandError> {
        use crate::command::CommandError;
        let mut width = metrics.default_column_width.max(metrics.min_column_width);
        for panel_id in panels {
            let Some(panel) = self.panels.get(*panel_id) else {
                return Err(CommandError::UnknownPanel(*panel_id));
            };
            width = width.max(panel.constraints().min_size().width);
        }
        if !width.is_finite() || width <= 0.0 {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "column width".to_owned(),
                reason: "must be finite and positive".to_owned(),
            });
        }
        Ok(width)
    }

    fn resolve_column_ref(
        &self,
        column_ref: crate::command::ColumnRef,
    ) -> Result<(usize, usize), crate::command::CommandError> {
        use crate::command::CommandError;

        let Some(area_index) = self
            .dock_areas
            .iter()
            .position(|area| area.side == column_ref.side)
        else {
            return Err(CommandError::UnknownColumn(column_ref));
        };
        if self.dock_areas[area_index]
            .columns
            .get(column_ref.index)
            .is_none()
        {
            return Err(CommandError::UnknownColumn(column_ref));
        }
        Ok((area_index, column_ref.index))
    }

    fn minimum_group_height(
        &self,
        group_id: NodeId,
        metrics: &DockMetrics,
    ) -> Result<f32, crate::command::CommandError> {
        use crate::command::CommandError;

        let Some(node) = self.nodes.get(group_id) else {
            return Err(CommandError::UnknownGroup(group_id));
        };
        let Node::Tabs(tabs) = node else {
            return Err(CommandError::GroupNotTabs(group_id));
        };

        let mut minimum = metrics.min_group_height;
        for panel_id in &tabs.panels {
            let Some(panel) = self.panels.get(*panel_id) else {
                return Err(CommandError::UnknownPanel(*panel_id));
            };
            let height = panel.constraints().min_size().height;
            if !height.is_finite() || height < 0.0 {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "panel minimum height".to_owned(),
                    reason: "must be finite and non-negative".to_owned(),
                });
            }
            minimum = minimum.max(height);
        }
        if !minimum.is_finite() || minimum < 0.0 {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "group minimum height".to_owned(),
                reason: "must be finite and non-negative".to_owned(),
            });
        }
        Ok(minimum)
    }

    fn minimum_column_width(
        &self,
        area_index: usize,
        column_index: usize,
        metrics: &DockMetrics,
    ) -> f32 {
        match self.dock_areas[area_index].columns[column_index].mode {
            ColumnMode::Expanded => metrics.min_column_width,
            ColumnMode::IconsAndLabels | ColumnMode::IconsOnly => metrics.rail_width,
        }
    }

    fn resize_groups(
        &mut self,
        column_ref: crate::command::ColumnRef,
        boundary: usize,
        delta: f32,
        metrics: &DockMetrics,
        actual_value: &mut Option<f32>,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        if !delta.is_finite() {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "delta".to_owned(),
                reason: "must be finite".to_owned(),
            });
        }
        let (area_index, column_index) = self.resolve_column_ref(column_ref)?;
        let available_height = self.canvas.bounds.size.height;
        if !available_height.is_finite() || available_height <= 0.0 {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "canvas height".to_owned(),
                reason: "must be finite and positive before resizing group boundaries".to_owned(),
            });
        }

        let (first_group, second_group, first_ratio, second_ratio) = {
            let column = &self.dock_areas[area_index].columns[column_index];
            if boundary >= column.groups.len().saturating_sub(1) {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "group boundary".to_owned(),
                    reason: format!(
                        "boundary {boundary} must identify two adjacent groups in column {column_ref:?}"
                    ),
                });
            }
            if column.group_ratios.len() != column.groups.len() {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "group_ratios".to_owned(),
                    reason: "there must be one ratio for every group".to_owned(),
                });
            }
            let first_ratio = column.group_ratios[boundary];
            let second_ratio = column.group_ratios[boundary + 1];
            if !first_ratio.is_finite()
                || first_ratio < 0.0
                || !second_ratio.is_finite()
                || second_ratio < 0.0
            {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "group_ratios".to_owned(),
                    reason: "adjacent group ratios must be finite and non-negative".to_owned(),
                });
            }
            (
                column.groups[boundary],
                column.groups[boundary + 1],
                first_ratio,
                second_ratio,
            )
        };

        let first_height = first_ratio * available_height;
        let second_height = second_ratio * available_height;
        let pair_height = first_height + second_height;
        if !first_height.is_finite() || !second_height.is_finite() || !pair_height.is_finite() {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "group height".to_owned(),
                reason: "current group heights must be finite".to_owned(),
            });
        }

        let first_minimum = self.minimum_group_height(first_group, metrics)?;
        let second_minimum = self.minimum_group_height(second_group, metrics)?;
        let minimum_pair_height = first_minimum + second_minimum;
        if !minimum_pair_height.is_finite() || minimum_pair_height > pair_height + PIXEL_TOLERANCE {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "group minimum heights".to_owned(),
                reason: format!(
                    "adjacent groups need at least {minimum_pair_height:.3} logical px, but their combined allocation is {pair_height:.3}"
                ),
            });
        }

        // Positive delta grows the first (upper) group and shrinks the next group.
        let minimum_delta = first_minimum - first_height;
        let maximum_delta = second_height - second_minimum;
        if !minimum_delta.is_finite() || !maximum_delta.is_finite() || minimum_delta > maximum_delta
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "delta bounds".to_owned(),
                reason: "group minimums leave no valid splitter position".to_owned(),
            });
        }
        let applied_delta = if delta < minimum_delta {
            minimum_delta
        } else if delta > maximum_delta {
            maximum_delta
        } else {
            delta
        };
        let updated_first_height = first_height + applied_delta;
        let updated_second_height = second_height - applied_delta;
        if !updated_first_height.is_finite()
            || !updated_second_height.is_finite()
            || updated_first_height < 0.0
            || updated_second_height < 0.0
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "group height".to_owned(),
                reason: "resized group heights must be finite and non-negative".to_owned(),
            });
        }

        let column = &mut self.dock_areas[area_index].columns[column_index];
        column.group_ratios[boundary] = updated_first_height / available_height;
        column.group_ratios[boundary + 1] = updated_second_height / available_height;
        *actual_value = Some(applied_delta);
        Ok(vec![first_group, second_group])
    }

    fn resize_column(
        &mut self,
        column_ref: crate::command::ColumnRef,
        requested_width: f32,
        metrics: &DockMetrics,
        actual_value: &mut Option<f32>,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        if !requested_width.is_finite() || requested_width < 0.0 {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "width".to_owned(),
                reason: "must be finite and non-negative".to_owned(),
            });
        }
        let (area_index, column_index) = self.resolve_column_ref(column_ref)?;
        let minimum_width = self.minimum_column_width(area_index, column_index, metrics);
        if !minimum_width.is_finite() || minimum_width <= 0.0 {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "minimum column width".to_owned(),
                reason: "must be finite and positive".to_owned(),
            });
        }

        // When current Canvas geometry is available, enlarging this column must not
        // leave the document region narrower than min_document_width. Before geometry
        // has been assigned (zero width), the constraint is deferred to the geometry stage.
        let current_width = self.dock_areas[area_index].columns[column_index].width;
        let canvas_width = self.canvas.bounds.size.width;
        let maximum_width = if canvas_width == 0.0 {
            None
        } else if !canvas_width.is_finite()
            || canvas_width < 0.0
            || !current_width.is_finite()
            || current_width <= 0.0
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "canvas/column width".to_owned(),
                reason: "known geometry and current column width must be finite and non-negative/positive".to_owned(),
            });
        } else {
            let available_growth = (canvas_width - metrics.min_document_width).max(0.0);
            let maximum = current_width + available_growth;
            if !maximum.is_finite() || maximum + PIXEL_TOLERANCE < minimum_width {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "column width".to_owned(),
                    reason:
                        "current geometry cannot satisfy both column and minimum document widths"
                            .to_owned(),
                });
            }
            Some(maximum)
        };

        let requested_applied_width = requested_width.max(minimum_width);
        let applied_width = maximum_width
            .map(|maximum| requested_applied_width.min(maximum))
            .unwrap_or(requested_applied_width);
        if !applied_width.is_finite() || applied_width + PIXEL_TOLERANCE < minimum_width {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "width".to_owned(),
                reason: "resulting width must be finite and satisfy the column minimum".to_owned(),
            });
        }

        let column = &mut self.dock_areas[area_index].columns[column_index];
        column.width = applied_width;
        let changed = column.groups.clone();
        *actual_value = Some(applied_width);
        Ok(changed)
    }

    fn resize_column_boundary(
        &mut self,
        first_ref: crate::command::ColumnRef,
        second_ref: crate::command::ColumnRef,
        delta: f32,
        metrics: &DockMetrics,
        actual_value: &mut Option<f32>,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        if !delta.is_finite() {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "delta".to_owned(),
                reason: "must be finite".to_owned(),
            });
        }
        if first_ref.side != second_ref.side
            || first_ref.index.checked_add(1) != Some(second_ref.index)
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "columns".to_owned(),
                reason: "references must identify two adjacent columns in their existing order"
                    .to_owned(),
            });
        }
        let (area_index, first_index) = self.resolve_column_ref(first_ref)?;
        let (second_area_index, second_index) = self.resolve_column_ref(second_ref)?;
        if area_index != second_area_index || second_index != first_index + 1 {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "columns".to_owned(),
                reason: "references must identify two adjacent columns in the same dock area"
                    .to_owned(),
            });
        }

        let (first_width, second_width, first_groups, second_groups) = {
            let columns = &self.dock_areas[area_index].columns;
            let first = &columns[first_index];
            let second = &columns[second_index];
            if !first.width.is_finite()
                || first.width <= 0.0
                || !second.width.is_finite()
                || second.width <= 0.0
            {
                return Err(CommandError::InvalidCommandParameter {
                    parameter: "column width".to_owned(),
                    reason: "current widths must be finite and positive".to_owned(),
                });
            }
            (
                first.width,
                second.width,
                first.groups.clone(),
                second.groups.clone(),
            )
        };
        let combined_width = first_width + second_width;
        if !combined_width.is_finite() {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "combined column width".to_owned(),
                reason: "must be finite".to_owned(),
            });
        }

        let first_minimum = self.minimum_column_width(area_index, first_index, metrics);
        let second_minimum = self.minimum_column_width(area_index, second_index, metrics);
        let minimum_pair_width = first_minimum + second_minimum;
        if !minimum_pair_width.is_finite() || minimum_pair_width > combined_width + PIXEL_TOLERANCE
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "column minimum widths".to_owned(),
                reason: "adjacent columns cannot both satisfy their minimum widths without changing their combined width".to_owned(),
            });
        }

        // Positive delta grows the first column and shrinks the second by the same amount.
        let minimum_delta = first_minimum - first_width;
        let maximum_delta = second_width - second_minimum;
        if !minimum_delta.is_finite() || !maximum_delta.is_finite() || minimum_delta > maximum_delta
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "delta bounds".to_owned(),
                reason: "column minimums leave no valid splitter position".to_owned(),
            });
        }
        let applied_delta = if delta < minimum_delta {
            minimum_delta
        } else if delta > maximum_delta {
            maximum_delta
        } else {
            delta
        };
        let updated_first_width = first_width + applied_delta;
        let updated_second_width = second_width - applied_delta;
        if !updated_first_width.is_finite()
            || updated_first_width <= 0.0
            || !updated_second_width.is_finite()
            || updated_second_width <= 0.0
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "column width".to_owned(),
                reason: "resized widths must be finite and positive".to_owned(),
            });
        }

        let columns = &mut self.dock_areas[area_index].columns;
        columns[first_index].width = updated_first_width;
        columns[second_index].width = updated_second_width;
        *actual_value = Some(applied_delta);
        let mut changed = first_groups;
        changed.extend(second_groups);
        Ok(changed)
    }

    /// When Canvas geometry is known, ensure the dock column's group minimum heights
    /// fit the available vertical span. A zero-sized Canvas is the pre-geometry state,
    /// so feasibility is deferred until R4 assigns window-relative rectangles.
    fn ensure_column_fits_min_height(
        &self,
        area_index: usize,
        column_index: usize,
        metrics: &DockMetrics,
    ) -> Result<(), crate::command::CommandError> {
        use crate::command::CommandError;

        let available = self.canvas.bounds.size.height;
        if !available.is_finite() || available <= 0.0 {
            return Ok(());
        }
        let Some(area) = self.dock_areas.get(area_index) else {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "dock area".to_owned(),
                reason: "area disappeared while checking split dimensions".to_owned(),
            });
        };
        let Some(column) = area.columns.get(column_index) else {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "column".to_owned(),
                reason: "column disappeared while checking split dimensions".to_owned(),
            });
        };

        let mut required_height = 0.0_f32;
        let mut non_empty_groups = 0usize;
        for group_id in &column.groups {
            let Some(Node::Tabs(tabs)) = self.nodes.get(*group_id) else {
                // The final validate call reports non-Tabs nodes in a Photoshop column.
                continue;
            };
            if tabs.panels.is_empty() {
                // This group is removed by normalization immediately after the command.
                continue;
            }
            let mut group_minimum = metrics.min_group_height;
            for panel_id in &tabs.panels {
                let Some(panel) = self.panels.get(*panel_id) else {
                    continue;
                };
                let min_height = panel.constraints().min_size().height;
                if !min_height.is_finite() || min_height < 0.0 {
                    return Err(CommandError::InvalidCommandParameter {
                        parameter: "panel minimum height".to_owned(),
                        reason: "must be finite and non-negative".to_owned(),
                    });
                }
                group_minimum = group_minimum.max(min_height);
            }
            required_height += group_minimum;
            non_empty_groups += 1;
        }
        // The separator rows consume physical height too; omitting them allowed a
        // direct side-drop command to pass with groups below their minimum allocations.
        required_height += metrics.splitter_thickness.max(0.0)
            * non_empty_groups.saturating_sub(1) as f32;

        if !required_height.is_finite() || required_height > available + PIXEL_TOLERANCE {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "column height".to_owned(),
                reason: format!(
                    "split requires at least {required_height:.3} logical px but only {available:.3} are available"
                ),
            });
        }
        Ok(())
    }

    fn find_dock_group_location(&self, group: NodeId) -> Option<(DockSide, usize, usize)> {
        self.dock_areas.iter().find_map(|area| {
            area.columns
                .iter()
                .enumerate()
                .find_map(|(column_index, column)| {
                    column
                        .groups
                        .iter()
                        .position(|candidate| *candidate == group)
                        .map(|group_index| (area.side, column_index, group_index))
                })
        })
    }

    fn find_dock_group_for_panel(&self, panel: crate::ids::PanelId) -> Option<(NodeId, usize)> {
        self.nodes.iter().find_map(|(node_id, node)| {
            let Node::Tabs(tabs) = node else {
                return None;
            };
            tabs.panels
                .iter()
                .position(|candidate| *candidate == panel)
                .map(|index| (node_id, index))
        })
    }

    fn remember_panel_location(&mut self, panel: crate::ids::PanelId, group: NodeId) {
        let Some(key) = self
            .panels
            .get(panel)
            .map(|instance| instance.key().clone())
        else {
            return;
        };
        let Some((side, column_index, group_index)) = self.find_dock_group_location(group) else {
            return;
        };
        self.last_panel_locations
            .retain(|(saved_key, _, _, _)| saved_key != &key);
        self.last_panel_locations
            .push((key, side, column_index, group_index));
    }

    fn remove_panel_from_tabs(tabs: &mut Tabs, index: usize) -> crate::ids::PanelId {
        let removed = tabs.panels.remove(index);
        if tabs.panels.is_empty() {
            tabs.active_index = 0;
        } else if index < tabs.active_index {
            tabs.active_index -= 1;
        } else if index == tabs.active_index {
            tabs.active_index = tabs.active_index.min(tabs.panels.len() - 1);
        }
        removed
    }

    fn insert_panel_into_tabs(
        tabs: &mut Tabs,
        panel: crate::ids::PanelId,
        index: usize,
        activate: bool,
    ) {
        let old_active = tabs.active_index;
        tabs.panels.insert(index, panel);
        if activate {
            tabs.active_index = index;
        } else if index <= old_active {
            tabs.active_index = old_active + 1;
        }
    }

    fn close_panel(
        &mut self,
        panel: crate::ids::PanelId,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        if !self.panels.contains_key(panel) {
            return Err(CommandError::UnknownPanel(panel));
        }

        if let Some((group, index)) = self.find_dock_group_for_panel(panel) {
            self.remember_panel_location(panel, group);
            let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                return Err(CommandError::GroupNotTabs(group));
            };
            Self::remove_panel_from_tabs(tabs, index);
            self.panels.remove(panel);
            return Ok(vec![group]);
        }

        let floating_location = self
            .floating_groups
            .iter()
            .find_map(|(floating_id, floating)| {
                floating
                    .panels
                    .iter()
                    .position(|candidate| *candidate == panel)
                    .map(|index| (floating_id, index))
            });
        if let Some((floating_id, index)) = floating_location {
            if let Some(floating) = self.floating_groups.get_mut(floating_id) {
                floating.panels.remove(index);
                if floating.panels.is_empty() {
                    self.floating_groups.remove(floating_id);
                } else if index < floating.active_index {
                    floating.active_index -= 1;
                } else if index == floating.active_index {
                    floating.active_index = floating.active_index.min(floating.panels.len() - 1);
                }
            }
            self.panels.remove(panel);
            return Ok(Vec::new());
        }

        Err(CommandError::PanelNotPlaced(panel))
    }

    fn close_group(&mut self, group: NodeId) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::CommandError;

        let Some(node) = self.nodes.get(group) else {
            return Err(CommandError::UnknownGroup(group));
        };
        let Node::Tabs(tabs) = node else {
            return Err(CommandError::GroupNotTabs(group));
        };
        let panel_ids = tabs.panels.clone();
        for panel_id in &panel_ids {
            self.remember_panel_location(*panel_id, group);
        }
        for panel_id in panel_ids {
            self.panels.remove(panel_id);
        }
        self.nodes.remove(group);
        Ok(vec![group])
    }

    /// Move one docked panel to a four-sided drop target by creating a new group.
    ///
    /// The command runs inside Layout::apply's candidate transaction, so every failure
    /// leaves the original layout untouched and successful changes are normalized/validated.
    fn move_panel_to_dock_side(
        &mut self,
        panel: crate::ids::PanelId,
        target: crate::command::DropTarget,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, DropTarget, DropZone};
        use crate::model::DockSide;

        if self.profile != Profile::Photoshop {
            return Err(CommandError::UnsupportedForProfile {
                command: "four-sided MovePanel".to_owned(),
                profile: self.profile,
            });
        }
        if !self.panels.contains_key(panel) {
            return Err(CommandError::UnknownPanel(panel));
        }
        let Some((source_group, source_index)) = self.find_dock_group_for_panel(panel) else {
            return Err(CommandError::PanelNotInDockGroup(panel));
        };

        match target {
            DropTarget::Group {
                group,
                zone: zone @ (DropZone::Top | DropZone::Bottom),
            } => {
                let Some((side, column_index, group_index)) =
                    self.find_dock_group_location(group)
                else {
                    return Err(CommandError::UnknownGroup(group));
                };
                if !matches!(self.nodes.get(group), Some(Node::Tabs(_))) {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                }
                let area_index = self
                    .dock_areas
                    .iter()
                    .position(|area| area.side == side)
                    .ok_or(CommandError::UnknownGroup(group))?;
                self.ensure_panels_fit_column(&[panel], area_index, column_index, metrics)?;

                let ratio = self.dock_areas[area_index].columns[column_index]
                    .group_ratios
                    .get(group_index)
                    .copied()
                    .ok_or_else(|| CommandError::InvalidCommandParameter {
                        parameter: "group_ratios".to_owned(),
                        reason: "target group has no matching height ratio".to_owned(),
                    })?;
                if !ratio.is_finite() || ratio < 0.0 {
                    return Err(CommandError::InvalidCommandParameter {
                        parameter: "group_ratios".to_owned(),
                        reason: "target group ratio must be finite and non-negative".to_owned(),
                    });
                }
                if self.dock_areas[area_index].columns[column_index].group_ratios.len()
                    != self.dock_areas[area_index].columns[column_index].groups.len()
                {
                    return Err(CommandError::InvalidCommandParameter {
                        parameter: "group_ratios".to_owned(),
                        reason: "there must be one ratio for every group".to_owned(),
                    });
                }

                let Some(Node::Tabs(source_tabs)) = self.nodes.get_mut(source_group) else {
                    return Err(CommandError::GroupNotTabs(source_group));
                };
                Self::remove_panel_from_tabs(source_tabs, source_index);
                let new_group = self.insert_node(Node::Tabs(Tabs {
                    panels: vec![panel],
                    active_index: 0,
                }));

                let insert_at = if zone == DropZone::Top {
                    group_index
                } else {
                    group_index + 1
                };
                let half = ratio * 0.5;
                let column = &mut self.dock_areas[area_index].columns[column_index];
                column.group_ratios[group_index] = half;
                column.groups.insert(insert_at, new_group);
                column.group_ratios.insert(insert_at, half);
                self.ensure_column_fits_min_height(area_index, column_index, metrics)?;
                Ok(vec![source_group, group, new_group])
            }
            DropTarget::Column { column, zone } => {
                let (area_index, column_index) = self.resolve_column_ref(column)?;
                match zone {
                    DropZone::Left | DropZone::Right => {
                        self.ensure_can_add_column(metrics)?;
                        let width = self.minimum_floating_dock_width(&[panel], metrics)?;
                        self.ensure_document_width_after_new_column(width, metrics)?;

                        let Some(Node::Tabs(source_tabs)) = self.nodes.get_mut(source_group) else {
                            return Err(CommandError::GroupNotTabs(source_group));
                        };
                        Self::remove_panel_from_tabs(source_tabs, source_index);
                        let new_group = self.insert_node(Node::Tabs(Tabs {
                            panels: vec![panel],
                            active_index: 0,
                        }));
                        let new_column = Column {
                            width,
                            mode: ColumnMode::Expanded,
                            groups: vec![new_group],
                            group_ratios: vec![1.0],
                        };
                        let insert_at = column_index + usize::from(zone == DropZone::Right);
                        self.dock_areas[area_index]
                            .columns
                            .insert(insert_at, new_column);
                        self.ensure_column_fits_min_height(area_index, insert_at, metrics)?;
                        Ok(vec![source_group, new_group])
                    }
                    DropZone::Top | DropZone::Bottom => {
                        self.ensure_panels_fit_column(&[panel], area_index, column_index, metrics)?;

                        let Some(Node::Tabs(source_tabs)) = self.nodes.get_mut(source_group) else {
                            return Err(CommandError::GroupNotTabs(source_group));
                        };
                        Self::remove_panel_from_tabs(source_tabs, source_index);
                        let new_group = self.insert_node(Node::Tabs(Tabs {
                            panels: vec![panel],
                            active_index: 0,
                        }));
                        let column = &mut self.dock_areas[area_index].columns[column_index];
                        let insert_at = if zone == DropZone::Top {
                            0
                        } else {
                            column.groups.len()
                        };
                        column.groups.insert(insert_at, new_group);
                        column.group_ratios.insert(insert_at, 1.0);
                        normalize_ratios(&mut column.group_ratios, column.groups.len());
                        self.ensure_column_fits_min_height(area_index, column_index, metrics)?;
                        Ok(vec![source_group, new_group])
                    }
                    DropZone::Center => Err(CommandError::InvalidDropTarget(
                        "a column Center zone is not a docking destination".to_owned(),
                    )),
                }
            }
            DropTarget::DockEdge { side } => {
                self.ensure_can_add_column(metrics)?;
                let width = self.minimum_floating_dock_width(&[panel], metrics)?;
                self.ensure_document_width_after_new_column(width, metrics)?;

                let Some(Node::Tabs(source_tabs)) = self.nodes.get_mut(source_group) else {
                    return Err(CommandError::GroupNotTabs(source_group));
                };
                Self::remove_panel_from_tabs(source_tabs, source_index);
                let new_group = self.insert_node(Node::Tabs(Tabs {
                    panels: vec![panel],
                    active_index: 0,
                }));
                let new_column = Column {
                    width,
                    mode: ColumnMode::Expanded,
                    groups: vec![new_group],
                    group_ratios: vec![1.0],
                };

                if let Some(area_index) =
                    self.dock_areas.iter().position(|area| area.side == side)
                {
                    let insert_at = if side == DockSide::Left {
                        0
                    } else {
                        self.dock_areas[area_index].columns.len()
                    };
                    self.dock_areas[area_index]
                        .columns
                        .insert(insert_at, new_column);
                    self.ensure_column_fits_min_height(area_index, insert_at, metrics)?;
                } else {
                    self.dock_areas.push(DockArea {
                        side,
                        columns: vec![new_column],
                    });
                    let area_index = self.dock_areas.len() - 1;
                    self.ensure_column_fits_min_height(area_index, 0, metrics)?;
                }
                Ok(vec![source_group, new_group])
            }
            _ => Err(CommandError::InvalidDropTarget(
                "four-sided MovePanel requires a Group, Column, or DockEdge side target"
                    .to_owned(),
            )),
        }
    }

    fn ensure_document_width_after_new_column(
        &self,
        new_column_width: f32,
        metrics: &DockMetrics,
    ) -> Result<(), crate::command::CommandError> {
        use crate::command::CommandError;

        let canvas_width = self.canvas.bounds.size.width;
        if canvas_width == 0.0 {
            // Geometry has not yet been assigned to Canvas; the hit-testing layer will
            // hide side targets when the live viewport cannot preserve the document minimum.
            return Ok(());
        }
        if !canvas_width.is_finite()
            || canvas_width < 0.0
            || !new_column_width.is_finite()
            || new_column_width <= 0.0
        {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "canvas/column width".to_owned(),
                reason: "known document width and new column width must be finite and valid"
                    .to_owned(),
            });
        }
        let remaining_width = canvas_width - new_column_width;
        if remaining_width + PIXEL_TOLERANCE < metrics.min_document_width {
            return Err(CommandError::InvalidCommandParameter {
                parameter: "document width".to_owned(),
                reason: format!(
                    "new column leaves only {remaining_width:.3} logical px for Canvas; at least {:.3} are required",
                    metrics.min_document_width
                ),
            });
        }
        Ok(())
    }

    fn move_panel_to_existing_group(
        &mut self,
        panel: crate::ids::PanelId,
        target: crate::command::DropTarget,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, DropTarget, DropZone};

        if !self.panels.contains_key(panel) {
            return Err(CommandError::UnknownPanel(panel));
        }

        let dock_source = self
            .find_dock_group_for_panel(panel)
            .filter(|(group, _)| self.find_dock_group_location(*group).is_some());
        let floating_source = self.floating_groups.iter().find_map(|(floating, group)| {
            group
                .panels
                .iter()
                .position(|candidate| *candidate == panel)
                .map(|index| (floating, index))
        });
        if dock_source.is_none() && floating_source.is_none() {
            return Err(CommandError::PanelNotPlaced(panel));
        }

        let (target_group, requested_index) = match target {
            DropTarget::Group {
                group,
                zone: DropZone::Center,
            } => {
                let Some(Node::Tabs(tabs)) = self.nodes.get(group) else {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                };
                (group, tabs.panels.len())
            }
            DropTarget::TabInsertion { group, index } => {
                let Some(Node::Tabs(tabs)) = self.nodes.get(group) else {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                };
                if index > tabs.panels.len() {
                    return Err(CommandError::InvalidTabIndex { group, index });
                }
                (group, index)
            }
            _ => return Err(CommandError::InvalidDropTarget(
                "MovePanel supports existing dock groups and the Center zone of floating groups"
                    .to_owned(),
            )),
        };

        if let Some((source_group, source_index)) = dock_source {
            if source_group == target_group {
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(source_group) else {
                    return Err(CommandError::GroupNotTabs(source_group));
                };
                let mut insert_index = requested_index;
                if source_index < insert_index {
                    insert_index -= 1;
                }
                let moved = tabs.panels.remove(source_index);
                insert_index = insert_index.min(tabs.panels.len());
                tabs.panels.insert(insert_index, moved);
                // A tab dropped into a new position becomes the active tab, including
                // when it is reordered inside its current group.
                tabs.active_index = insert_index;
                return Ok(vec![source_group]);
            }
        }

        let mut changed_nodes = Vec::new();
        if let Some((source_group, source_index)) = dock_source {
            let Some(Node::Tabs(source_tabs)) = self.nodes.get_mut(source_group) else {
                return Err(CommandError::GroupNotTabs(source_group));
            };
            let active_source = source_tabs.panels.get(source_tabs.active_index).copied();
            Self::remove_panel_from_tabs(source_tabs, source_index);
            if let Some(active_source) = active_source.filter(|active| *active != panel) {
                if let Some(index) = source_tabs
                    .panels
                    .iter()
                    .position(|candidate| *candidate == active_source)
                {
                    source_tabs.active_index = index;
                }
            }
            changed_nodes.push(source_group);
        } else if let Some((source_id, source_index)) = floating_source {
            if let Some(source) = self.floating_groups.get_mut(source_id) {
                source.panels.remove(source_index);
                if source.panels.is_empty() {
                    self.floating_groups.remove(source_id);
                } else if source_index < source.active_index {
                    source.active_index -= 1;
                } else if source_index == source.active_index {
                    source.active_index = source.active_index.min(source.panels.len() - 1);
                }
            }
        }

        let Some(Node::Tabs(target_tabs)) = self.nodes.get_mut(target_group) else {
            return Err(CommandError::GroupNotTabs(target_group));
        };
        let insert_index = requested_index.min(target_tabs.panels.len());
        Self::insert_panel_into_tabs(target_tabs, panel, insert_index, true);
        changed_nodes.push(target_group);
        Ok(changed_nodes)
    }

    fn open_panel(
        &mut self,
        key: PanelKey,
        constraints: PanelConstraints,
        target: Option<crate::command::DropTarget>,
        metrics: &DockMetrics,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, DropTarget, DropZone};

        if constraints.is_singleton() {
            if let Some((existing_id, _)) = self.panels.iter().find(|(_, instance)| {
                instance.key() == &key && instance.constraints().is_singleton()
            }) {
                if let Some((group, index)) = self.find_dock_group_for_panel(existing_id) {
                    let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                        return Err(CommandError::GroupNotTabs(group));
                    };
                    tabs.active_index = index;
                    return Ok(vec![group]);
                }
                if let Some((floating_id, index)) =
                    self.floating_groups
                        .iter()
                        .find_map(|(floating_id, floating)| {
                            floating
                                .panels
                                .iter()
                                .position(|candidate| *candidate == existing_id)
                                .map(|index| (floating_id, index))
                        })
                {
                    if let Some(floating) = self.floating_groups.get_mut(floating_id) {
                        floating.active_index = index;
                    }
                    return Ok(Vec::new());
                }
            }
        }

        let panel = self.insert_panel(PanelInstance::new(key.clone(), constraints));
        if let Some(target) = target {
            return self.place_new_panel_at_explicit_target(panel, target);
        }

        let last_location = self
            .last_panel_locations
            .iter()
            .find(|(saved_key, _, _, _)| saved_key == &key)
            .map(|(_, side, column, group)| (*side, *column, *group));
        if let Some((side, column_index, group_index)) = last_location {
            if let Some(group) = self.group_at_location(side, column_index, group_index) {
                let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                    return Err(CommandError::GroupNotTabs(group));
                };
                let insert_index = tabs.panels.len();
                Self::insert_panel_into_tabs(tabs, panel, insert_index, true);
                return Ok(vec![group]);
            }
        }

        if let Some(group) = self.default_start_group() {
            let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
                return Err(CommandError::GroupNotTabs(group));
            };
            let insert_index = tabs.panels.len();
            Self::insert_panel_into_tabs(tabs, panel, insert_index, true);
            return Ok(vec![group]);
        }

        // The layout may have been closed down to no dock groups. Recreate the profile's
        // right-side starting location without duplicating any existing panel instance.
        let group = self.insert_node(Node::Tabs(Tabs {
            panels: vec![panel],
            active_index: 0,
        }));
        let right_area = self
            .dock_areas
            .iter()
            .position(|area| area.side == DockSide::Right);
        match right_area {
            Some(area_index) if !self.dock_areas[area_index].columns.is_empty() => {
                let column = &mut self.dock_areas[area_index].columns[0];
                column.groups.push(group);
                column.group_ratios.push(1.0);
                normalize_ratios(&mut column.group_ratios, column.groups.len());
            }
            Some(area_index) => {
                self.dock_areas[area_index].columns.push(Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![group],
                    group_ratios: vec![1.0],
                });
            }
            None => self.dock_areas.push(DockArea {
                side: DockSide::Right,
                columns: vec![Column {
                    width: metrics.default_column_width,
                    mode: ColumnMode::Expanded,
                    groups: vec![group],
                    group_ratios: vec![1.0],
                }],
            }),
        }
        Ok(vec![group])
    }

    fn place_new_panel_at_explicit_target(
        &mut self,
        panel: crate::ids::PanelId,
        target: crate::command::DropTarget,
    ) -> Result<Vec<NodeId>, crate::command::CommandError> {
        use crate::command::{CommandError, DropTarget, DropZone};

        let (group, index) = match target {
            DropTarget::Group {
                group,
                zone: DropZone::Center,
            } => {
                let Some(Node::Tabs(tabs)) = self.nodes.get(group) else {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                };
                (group, tabs.panels.len())
            }
            DropTarget::TabInsertion { group, index } => {
                let Some(Node::Tabs(tabs)) = self.nodes.get(group) else {
                    return if self.nodes.contains_key(group) {
                        Err(CommandError::GroupNotTabs(group))
                    } else {
                        Err(CommandError::UnknownGroup(group))
                    };
                };
                if index > tabs.panels.len() {
                    return Err(CommandError::InvalidTabIndex { group, index });
                }
                (group, index)
            }
            _ => {
                return Err(CommandError::InvalidDropTarget(
                    "R3.3 OpenPanel only supports insertion into an existing dock group".to_owned(),
                ));
            }
        };

        let Some(Node::Tabs(tabs)) = self.nodes.get_mut(group) else {
            return Err(CommandError::GroupNotTabs(group));
        };
        Self::insert_panel_into_tabs(tabs, panel, index, true);
        Ok(vec![group])
    }

    fn group_at_location(
        &self,
        side: DockSide,
        column_index: usize,
        group_index: usize,
    ) -> Option<NodeId> {
        let area = self.dock_areas.iter().find(|area| area.side == side)?;
        let column = area.columns.get(column_index)?;
        if column.groups.is_empty() {
            return None;
        }
        column
            .groups
            .get(group_index.min(column.groups.len() - 1))
            .copied()
            .filter(|group| matches!(self.nodes.get(*group), Some(Node::Tabs(_))))
    }

    fn default_start_group(&self) -> Option<NodeId> {
        self.dock_areas
            .iter()
            .filter(|area| area.side == DockSide::Right)
            .flat_map(|area| area.columns.iter())
            .flat_map(|column| column.groups.iter())
            .copied()
            .find(|group| matches!(self.nodes.get(*group), Some(Node::Tabs(_))))
            .or_else(|| {
                self.dock_areas
                    .iter()
                    .flat_map(|area| area.columns.iter())
                    .flat_map(|column| column.groups.iter())
                    .copied()
                    .find(|group| matches!(self.nodes.get(*group), Some(Node::Tabs(_))))
            })
    }

    /// Validate all model invariants and collect every detectable error.
    pub fn validate(&self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        if let Err(message) = metrics.validate() {
            errors.push(ValidationError::InvalidMetrics(message));
        }

        let mut node_parent_count: HashMap<NodeId, usize> = HashMap::new();
        let mut panel_placements: HashMap<PanelId, usize> = HashMap::new();
        let mut roots = Vec::new();
        let mut total_columns = 0usize;

        for (area_index, area) in self.dock_areas.iter().enumerate() {
            total_columns += area.columns.len();
            for (column_index, column) in area.columns.iter().enumerate() {
                if column.groups.is_empty() {
                    errors.push(ValidationError::EmptyColumn {
                        area_index,
                        column_index,
                    });
                }

                if !column.width.is_finite() || column.width <= 0.0 {
                    errors.push(ValidationError::InvalidColumnWidth {
                        area_index,
                        column_index,
                    });
                } else {
                    let minimum_width = match column.mode {
                        ColumnMode::Expanded => metrics.min_column_width,
                        ColumnMode::IconsAndLabels | ColumnMode::IconsOnly => metrics.rail_width,
                    };
                    if minimum_width.is_finite() && column.width + PIXEL_TOLERANCE < minimum_width {
                        errors.push(ValidationError::ColumnBelowMinimum {
                            area_index,
                            column_index,
                        });
                    }
                }

                if column.group_ratios.len() != column.groups.len() {
                    errors.push(ValidationError::ColumnRatioCountMismatch {
                        area_index,
                        column_index,
                    });
                }
                if !column
                    .group_ratios
                    .iter()
                    .all(|ratio| ratio.is_finite() && *ratio >= 0.0)
                {
                    errors.push(ValidationError::ColumnRatioInvalid {
                        area_index,
                        column_index,
                    });
                } else if column.group_ratios.len() == column.groups.len()
                    && !column.group_ratios.is_empty()
                    && !ratios_sum_to_one(&column.group_ratios)
                {
                    errors.push(ValidationError::ColumnRatiosDoNotSumToOne {
                        area_index,
                        column_index,
                    });
                }

                for group_id in &column.groups {
                    roots.push(*group_id);
                    if self.nodes.contains_key(*group_id) {
                        *node_parent_count.entry(*group_id).or_insert(0) += 1;
                    } else {
                        errors.push(ValidationError::NodeReferenceMissing(*group_id));
                    }

                    if self.profile == Profile::Photoshop
                        && matches!(self.nodes.get(*group_id), Some(Node::Split(_)))
                    {
                        errors.push(ValidationError::PhotoshopColumnGroupMustBeTabs {
                            area_index,
                            column_index,
                            group: *group_id,
                        });
                    }
                }
            }
        }

        if self.profile == Profile::Photoshop && total_columns > metrics.max_columns {
            errors.push(ValidationError::TooManyColumns {
                actual: total_columns,
                maximum: metrics.max_columns,
            });
        }

        for (node_id, node) in self.nodes.iter() {
            match node {
                Node::Split(split) => {
                    if split.children.len() < 2 {
                        errors.push(ValidationError::SplitTooFewChildren(node_id));
                    }
                    if split.ratios.len() != split.children.len() {
                        errors.push(ValidationError::SplitRatioCountMismatch(node_id));
                    }
                    if !split
                        .ratios
                        .iter()
                        .all(|ratio| ratio.is_finite() && *ratio >= 0.0)
                    {
                        errors.push(ValidationError::SplitRatioInvalid(node_id));
                    } else if split.ratios.len() == split.children.len()
                        && !split.ratios.is_empty()
                        && !ratios_sum_to_one(&split.ratios)
                    {
                        errors.push(ValidationError::SplitRatiosDoNotSumToOne(node_id));
                    }

                    if self.profile == Profile::Photoshop && split.axis != Axis::Vertical {
                        errors.push(ValidationError::PhotoshopSplitMustBeVertical(node_id));
                    }

                    for child in &split.children {
                        if self.nodes.contains_key(*child) {
                            *node_parent_count.entry(*child).or_insert(0) += 1;
                        } else {
                            errors.push(ValidationError::NodeReferenceMissing(*child));
                        }
                    }
                }
                Node::Tabs(tabs) => {
                    if tabs.panels.is_empty() {
                        errors.push(ValidationError::EmptyTabs(node_id));
                    }
                    if tabs.active_index >= tabs.panels.len() {
                        errors.push(ValidationError::ActiveTabOutOfBounds(node_id));
                    }
                    for panel_id in &tabs.panels {
                        *panel_placements.entry(*panel_id).or_insert(0) += 1;
                        if !self.panels.contains_key(*panel_id) {
                            errors.push(ValidationError::UnknownPanelReference {
                                node: node_id,
                                panel: *panel_id,
                            });
                        }
                    }
                }
            }
        }

        for (node_id, parent_count) in &node_parent_count {
            if *parent_count > 1 {
                errors.push(ValidationError::NodeHasMultipleParents(*node_id));
            }
        }

        let reachable = self.reachable_nodes(&roots);
        for node_id in self.nodes.keys() {
            if !reachable.contains(&node_id) {
                errors.push(ValidationError::UnreachableNode(node_id));
            }
        }

        let mut visit_state = HashMap::new();
        let mut reported_cycles = HashSet::new();
        for node_id in self.nodes.keys() {
            detect_cycles(
                node_id,
                &self.nodes,
                &mut visit_state,
                &mut reported_cycles,
                &mut errors,
            );
        }

        for (group_id, group) in self.floating_groups.iter() {
            if group.panels.is_empty() {
                errors.push(ValidationError::EmptyFloatingGroup(group_id));
            }
            if group.active_index >= group.panels.len() {
                errors.push(ValidationError::FloatingActiveTabOutOfBounds(group_id));
            }
            if !rect_has_positive_finite_size(group.bounds) {
                errors.push(ValidationError::InvalidFloatingBounds(group_id));
            }

            for panel_id in &group.panels {
                *panel_placements.entry(*panel_id).or_insert(0) += 1;
                if !self.panels.contains_key(*panel_id) {
                    errors.push(ValidationError::UnknownFloatingPanelReference {
                        group: group_id,
                        panel: *panel_id,
                    });
                }
            }
        }

        let mut panel_key_counts: HashMap<String, (usize, bool)> = HashMap::new();
        for (panel_id, panel) in self.panels.iter() {
            if !panel_constraints_are_valid(panel.constraints()) {
                errors.push(ValidationError::InvalidPanelConstraints(panel_id));
            }

            let count = panel_placements.get(&panel_id).copied().unwrap_or(0);
            if count == 0 {
                errors.push(ValidationError::PanelNotPlaced(panel_id));
            } else if count > 1 {
                errors.push(ValidationError::PanelPlacedMoreThanOnce(panel_id));
            }

            let key = panel.key().as_str().to_owned();
            let entry = panel_key_counts.entry(key).or_insert((0, false));
            entry.0 += 1;
            entry.1 |= panel.constraints().is_singleton();
        }

        for (key, (count, has_singleton)) in panel_key_counts {
            if count > 1 && has_singleton {
                errors.push(ValidationError::SingletonPanelDuplicated(key));
            }
        }

        // Unknown IDs can also be repeated in malformed references; report duplicate
        // placement once for every ID, irrespective of whether it exists in the panel arena.
        for (panel_id, count) in panel_placements {
            if count > 1 && !self.panels.contains_key(panel_id) {
                errors.push(ValidationError::PanelPlacedMoreThanOnce(panel_id));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn reachable_nodes(&self, roots: &[NodeId]) -> HashSet<NodeId> {
        let mut reachable = HashSet::new();
        let mut pending = roots.to_vec();

        while let Some(node_id) = pending.pop() {
            if !self.nodes.contains_key(node_id) || !reachable.insert(node_id) {
                continue;
            }
            if let Some(Node::Split(split)) = self.nodes.get(node_id) {
                pending.extend(split.children.iter().copied());
            }
        }
        reachable
    }

    /// Normalize the layout on a candidate copy and commit it only if the result validates.
    ///
    /// Command handling should call this only after a command has otherwise succeeded. The
    /// method is crate-private so Layout::apply remains the only public mutation path.
    pub(crate) fn normalize(&mut self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut candidate = self.clone();
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(())
    }

    fn normalize_in_place(&mut self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        if let Err(message) = metrics.validate() {
            return Err(vec![ValidationError::InvalidMetrics(message)]);
        }

        let mut removed_nodes = HashSet::new();
        let empty_tabs: Vec<NodeId> = self
            .nodes
            .iter()
            .filter_map(|(node_id, node)| match node {
                Node::Tabs(tabs) if tabs.panels.is_empty() => Some(node_id),
                _ => None,
            })
            .collect();
        for node_id in empty_tabs {
            self.nodes.remove(node_id);
            removed_nodes.insert(node_id);
        }

        loop {
            let mut changed = false;
            let node_ids: Vec<NodeId> = self.nodes.keys().collect();

            // Remove children that were empty groups or empty splits from the previous pass.
            for node_id in &node_ids {
                if let Some(Node::Split(split)) = self.nodes.get_mut(*node_id) {
                    let old_len = split.children.len();
                    if split.ratios.len() == split.children.len() {
                        let mut children = Vec::with_capacity(old_len);
                        let mut ratios = Vec::with_capacity(old_len);
                        for (child, ratio) in split
                            .children
                            .iter()
                            .copied()
                            .zip(split.ratios.iter().copied())
                        {
                            if !removed_nodes.contains(&child) {
                                children.push(child);
                                ratios.push(ratio);
                            }
                        }
                        split.children = children;
                        split.ratios = ratios;
                    } else {
                        split
                            .children
                            .retain(|child| !removed_nodes.contains(child));
                    }
                    changed |= split.children.len() != old_len;
                }
            }

            let mut collapsed: HashMap<NodeId, NodeId> = HashMap::new();
            let mut empty_splits = Vec::new();
            for node_id in &node_ids {
                if let Some(Node::Split(split)) = self.nodes.get(*node_id) {
                    if split.children.is_empty() {
                        empty_splits.push(*node_id);
                    } else if split.children.len() == 1 && split.children[0] != *node_id {
                        collapsed.insert(*node_id, split.children[0]);
                    }
                }
            }

            for node_id in &empty_splits {
                removed_nodes.insert(*node_id);
            }

            for area in &mut self.dock_areas {
                for column in &mut area.columns {
                    let old_len = column.groups.len();
                    rewrite_node_references(
                        &mut column.groups,
                        &mut column.group_ratios,
                        &removed_nodes,
                        &collapsed,
                    );
                    changed |= column.groups.len() != old_len;
                }
            }

            let remaining_ids: Vec<NodeId> = self.nodes.keys().collect();
            for node_id in &remaining_ids {
                if let Some(Node::Split(split)) = self.nodes.get_mut(*node_id) {
                    let old_len = split.children.len();
                    rewrite_node_references(
                        &mut split.children,
                        &mut split.ratios,
                        &removed_nodes,
                        &collapsed,
                    );
                    changed |= split.children.len() != old_len;
                }
            }

            for node_id in empty_splits {
                changed |= self.nodes.remove(node_id).is_some();
            }
            for node_id in collapsed.keys().copied().collect::<Vec<_>>() {
                changed |= self.nodes.remove(node_id).is_some();
                removed_nodes.insert(node_id);
            }

            if !changed {
                break;
            }
        }

        for area in &mut self.dock_areas {
            area.columns.retain(|column| !column.groups.is_empty());
            for column in &mut area.columns {
                normalize_ratios(&mut column.group_ratios, column.groups.len());
            }
        }
        self.dock_areas.retain(|area| !area.columns.is_empty());

        for node in self.nodes.values_mut() {
            if let Node::Split(split) = node {
                normalize_ratios(&mut split.ratios, split.children.len());
            }
        }

        let roots: Vec<NodeId> = self
            .dock_areas
            .iter()
            .flat_map(|area| area.columns.iter())
            .flat_map(|column| column.groups.iter().copied())
            .collect();
        let reachable = self.reachable_nodes(&roots);
        let unreachable: Vec<NodeId> = self
            .nodes
            .keys()
            .filter(|node_id| !reachable.contains(node_id))
            .collect();
        for node_id in unreachable {
            self.nodes.remove(node_id);
        }

        self.validate(metrics)
    }

    // Crate-internal construction/mutation hooks. Public callers can only read the model;
    // user-visible mutation will be routed through Layout::apply(DockCommand) in stage 3.
    pub(crate) fn insert_node(&mut self, node: Node) -> NodeId {
        self.nodes.insert(node)
    }

    pub(crate) fn insert_panel(&mut self, panel: PanelInstance) -> PanelId {
        self.panels.insert(panel)
    }

    pub(crate) fn push_dock_area(&mut self, area: DockArea) {
        self.dock_areas.push(area);
    }

    pub(crate) fn insert_floating_group(&mut self, group: FloatingGroup) -> FloatingGroupId {
        self.floating_groups.insert(group)
    }

    pub(crate) fn set_canvas(&mut self, canvas: CanvasState) {
        self.canvas = canvas;
    }

    pub(crate) fn panel_mut(&mut self, id: PanelId) -> Option<&mut PanelInstance> {
        self.panels.get_mut(id)
    }

    pub(crate) fn advance_version(&mut self) -> Result<u64, &'static str> {
        let next = self
            .version
            .checked_add(1)
            .ok_or("layout version exhausted")?;
        self.version = next;
        Ok(next)
    }
}

fn ratios_sum_to_one(ratios: &[f32]) -> bool {
    let sum: f32 = ratios.iter().sum();
    sum.is_finite() && (sum - 1.0).abs() <= RATIO_TOLERANCE
}

fn rect_has_positive_finite_size(rect: crate::geometry::Rect) -> bool {
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
        && rect.size.width > 0.0
        && rect.size.height > 0.0
}

fn dimension_is_valid(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

fn panel_constraints_are_valid(constraints: &PanelConstraints) -> bool {
    let min = constraints.min_size();
    let preferred = constraints.preferred_size();
    let max = constraints.max_size();
    [
        min.width,
        min.height,
        preferred.width,
        preferred.height,
        max.width,
        max.height,
    ]
    .into_iter()
    .all(dimension_is_valid)
        && min.width <= preferred.width + PIXEL_TOLERANCE
        && min.height <= preferred.height + PIXEL_TOLERANCE
        && preferred.width <= max.width + PIXEL_TOLERANCE
        && preferred.height <= max.height + PIXEL_TOLERANCE
}

fn detect_cycles(
    node_id: NodeId,
    nodes: &SlotMap<NodeId, Node>,
    state: &mut HashMap<NodeId, u8>,
    reported_cycles: &mut HashSet<NodeId>,
    errors: &mut Vec<ValidationError>,
) {
    match state.get(&node_id).copied() {
        Some(1) => {
            if reported_cycles.insert(node_id) {
                errors.push(ValidationError::NodeCycle(node_id));
            }
            return;
        }
        Some(2) => return,
        _ => {}
    }

    state.insert(node_id, 1);
    if let Some(Node::Split(split)) = nodes.get(node_id) {
        for child in &split.children {
            if nodes.contains_key(*child) {
                detect_cycles(*child, nodes, state, reported_cycles, errors);
            }
        }
    }
    state.insert(node_id, 2);
}

fn resolve_node_reference(
    original: NodeId,
    removed: &HashSet<NodeId>,
    collapsed: &HashMap<NodeId, NodeId>,
) -> Option<NodeId> {
    let mut current = original;
    let mut seen = HashSet::new();

    loop {
        if !seen.insert(current) {
            // Do not try to repair a cycle by guessing; the final validation will reject it.
            return Some(original);
        }
        if let Some(next) = collapsed.get(&current) {
            if *next != current {
                current = *next;
                continue;
            }
        }
        if removed.contains(&current) {
            return None;
        }
        return Some(current);
    }
}

fn rewrite_node_references(
    node_ids: &mut Vec<NodeId>,
    ratios: &mut Vec<f32>,
    removed: &HashSet<NodeId>,
    collapsed: &HashMap<NodeId, NodeId>,
) {
    let old_ids = std::mem::take(node_ids);
    let old_ratios = std::mem::take(ratios);
    let ratios_were_aligned = old_ids.len() == old_ratios.len();
    let mut new_ids = Vec::with_capacity(old_ids.len());
    let mut new_ratios = Vec::with_capacity(old_ratios.len());

    for (index, old_id) in old_ids.iter().copied().enumerate() {
        if let Some(new_id) = resolve_node_reference(old_id, removed, collapsed) {
            new_ids.push(new_id);
            if ratios_were_aligned {
                new_ratios.push(old_ratios[index]);
            }
        }
    }

    *node_ids = new_ids;
    *ratios = if ratios_were_aligned {
        new_ratios
    } else {
        old_ratios
    };
}

fn normalize_ratios(ratios: &mut Vec<f32>, expected_len: usize) {
    if expected_len == 0 {
        ratios.clear();
        return;
    }

    let sum: f32 = ratios.iter().sum();
    let usable = ratios.len() == expected_len
        && ratios
            .iter()
            .all(|ratio| ratio.is_finite() && *ratio >= 0.0)
        && sum.is_finite()
        && sum > 0.0;

    if !usable {
        *ratios = vec![1.0 / expected_len as f32; expected_len];
        return;
    }

    for ratio in ratios {
        *ratio /= sum;
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;
    use crate::{
        Axis, CanvasState, Column, ColumnMode, DockArea, DockMetrics, DockSide, FloatingGroup,
        Node, PanelConstraints, PanelInstance, PanelKey, Point, Profile, Rect, Size, Split, Tabs,
        ValidationError,
    };
    use slotmap::Key;

    fn metrics() -> DockMetrics {
        DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        }
    }

    fn empty_layout() -> Layout {
        Layout::new(
            Profile::Photoshop,
            CanvasState {
                bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, 800.0)),
            },
        )
    }

    fn initial_layout() -> Layout {
        Layout::initial_photoshop(&metrics()).expect("valid metrics")
    }

    fn tab_group_id(layout: &Layout, column_index: usize, group_index: usize) -> crate::NodeId {
        layout.dock_areas[0].columns[column_index].groups[group_index]
    }

    #[test]
    fn starts_with_empty_arenas_and_version_zero() {
        let layout = empty_layout();
        assert!(layout.nodes().is_empty());
        assert!(layout.panels().is_empty());
        assert!(layout.dock_areas().is_empty());
        assert!(layout.floating_groups().is_empty());
        assert_eq!(layout.profile(), Profile::Photoshop);
        assert_eq!(layout.version(), 0);
    }

    #[test]
    fn version_advances_only_when_a_change_is_recorded() {
        let mut layout = empty_layout();
        assert_eq!(layout.advance_version(), Ok(1));
        assert_eq!(layout.advance_version(), Ok(2));
        assert_eq!(layout.version(), 2);
    }

    #[test]
    fn version_overflow_is_reported_without_wrapping() {
        let mut layout = empty_layout();
        layout.version = u64::MAX;
        assert!(layout.advance_version().is_err());
        assert_eq!(layout.version(), u64::MAX);
    }

    #[test]
    fn initial_photoshop_layout_has_canvas_one_right_column_and_two_groups() {
        let layout = initial_layout();

        assert_eq!(layout.profile(), Profile::Photoshop);
        assert_eq!(layout.version(), 0);
        assert!(layout.floating_groups().is_empty());
        assert_eq!(layout.dock_areas().len(), 1);

        let area = &layout.dock_areas()[0];
        assert_eq!(area.side, DockSide::Right);
        assert_eq!(area.columns.len(), 1);

        let column = &area.columns[0];
        assert!((column.width - 280.0).abs() < 1e-3);
        assert_eq!(column.mode, ColumnMode::Expanded);
        assert_eq!(column.groups.len(), 2);
        assert_eq!(column.group_ratios, vec![0.5, 0.5]);

        let layers_tabs = match layout.nodes().get(column.groups[0]) {
            Some(Node::Tabs(tabs)) => tabs,
            other => panic!("expected first column group to be Tabs, got {other:?}"),
        };
        let properties_tabs = match layout.nodes().get(column.groups[1]) {
            Some(Node::Tabs(tabs)) => tabs,
            other => panic!("expected second column group to be Tabs, got {other:?}"),
        };

        assert_eq!(layers_tabs.panels.len(), 2);
        assert_eq!(layers_tabs.active_index, 0);
        assert_eq!(
            layout
                .panels()
                .get(layers_tabs.panels[0])
                .map(|panel| panel.key().as_str()),
            Some("layers")
        );
        assert_eq!(
            layout
                .panels()
                .get(layers_tabs.panels[1])
                .map(|panel| panel.key().as_str()),
            Some("color")
        );

        assert_eq!(properties_tabs.panels.len(), 2);
        assert_eq!(properties_tabs.active_index, 0);
        assert_eq!(
            layout
                .panels()
                .get(properties_tabs.panels[0])
                .map(|panel| panel.key().as_str()),
            Some("properties")
        );
        assert_eq!(
            layout
                .panels()
                .get(properties_tabs.panels[1])
                .map(|panel| panel.key().as_str()),
            Some("history")
        );

        assert_eq!(layout.panels().len(), 4);
        assert!(
            layout
                .panels()
                .values()
                .all(|panel| panel.constraints().is_singleton())
        );
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn validation_collects_multiple_errors_instead_of_stopping_at_the_first() {
        let mut layout = initial_layout();
        let group_id = tab_group_id(&layout, 0, 0);

        layout.dock_areas[0].columns[0].width = -1.0;
        layout.dock_areas[0].columns[0].group_ratios = vec![0.5];
        if let Some(Node::Tabs(tabs)) = layout.nodes.get_mut(group_id) {
            tabs.active_index = 99;
        }

        let errors = layout
            .validate(&metrics())
            .expect_err("corrupt layout must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::InvalidColumnWidth { .. }))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::ColumnRatioCountMismatch { .. }))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::ActiveTabOutOfBounds(_)))
        );
    }

    #[test]
    fn validation_detects_duplicate_panel_placement() {
        let mut layout = initial_layout();
        let first_group = tab_group_id(&layout, 0, 0);
        let duplicate = match layout.nodes.get(first_group) {
            Some(Node::Tabs(tabs)) => tabs.panels[0],
            _ => unreachable!("initial layout group is Tabs"),
        };
        if let Some(Node::Tabs(tabs)) = layout.nodes.get_mut(first_group) {
            tabs.panels.push(duplicate);
        }

        let errors = layout
            .validate(&metrics())
            .expect_err("duplicate placement must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::PanelPlacedMoreThanOnce(_)))
        );
    }

    #[test]
    fn validation_detects_cycles_and_unreachable_nodes() {
        let mut layout = initial_layout();
        let cyclic = layout.insert_node(Node::Split(Split {
            axis: Axis::Vertical,
            children: Vec::new(),
            ratios: Vec::new(),
        }));
        if let Some(Node::Split(split)) = layout.nodes.get_mut(cyclic) {
            split.children.push(cyclic);
            split.ratios.push(1.0);
        }
        layout.dock_areas[0].columns[0].groups.push(cyclic);
        layout.dock_areas[0].columns[0].group_ratios.push(0.0);

        let errors = layout.validate(&metrics()).expect_err("cycle must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::NodeCycle(_)))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::SplitTooFewChildren(_)))
        );
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::PhotoshopColumnGroupMustBeTabs { .. }
        )));
    }

    #[test]
    fn a_column_with_one_group_is_valid_and_is_not_collapsed() {
        let mut layout = initial_layout();
        let (first_id, second_id) = {
            let groups = &layout.dock_areas[0].columns[0].groups;
            (groups[0], groups[1])
        };
        let second_panels = match layout.nodes.get(second_id) {
            Some(Node::Tabs(tabs)) => tabs.panels.clone(),
            _ => unreachable!("initial layout group is Tabs"),
        };
        if let Some(Node::Tabs(tabs)) = layout.nodes.get_mut(first_id) {
            tabs.panels.extend(second_panels);
        }
        layout.nodes.remove(second_id);
        layout.dock_areas[0].columns[0].groups = vec![first_id];
        layout.dock_areas[0].columns[0].group_ratios = vec![1.0];

        layout.dock_areas[0].columns[0].width = metrics().rail_width;
        layout.dock_areas[0].columns[0].mode = ColumnMode::IconsOnly;
        assert!(layout.validate(&metrics()).is_ok());
        layout
            .normalize(&metrics())
            .expect("normalization preserves a single-group column");
        assert_eq!(layout.dock_areas()[0].columns.len(), 1);
        assert_eq!(layout.dock_areas()[0].columns[0].groups.len(), 1);
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn validation_rejects_column_count_above_profile_maximum() {
        let mut layout = initial_layout();
        let template = layout.dock_areas[0].columns[0].clone();
        for _ in 0..metrics().max_columns {
            layout.dock_areas[0].columns.push(template.clone());
        }

        let errors = layout
            .validate(&metrics())
            .expect_err("too many columns must fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::TooManyColumns {
                actual: 5,
                maximum: 4
            }
        )));
    }

    #[test]
    fn normalization_removes_empty_tabs_and_renormalizes_column_ratios() {
        let mut layout = initial_layout();
        let empty_group = layout.insert_node(Node::Tabs(Tabs {
            panels: Vec::new(),
            active_index: 0,
        }));
        layout.dock_areas[0].columns[0].groups.push(empty_group);
        layout.dock_areas[0].columns[0].group_ratios = vec![0.4, 0.4, 0.2];

        assert!(layout.validate(&metrics()).is_err());
        layout
            .normalize(&metrics())
            .expect("empty group is removed");
        let column = &layout.dock_areas()[0].columns[0];
        assert_eq!(column.groups.len(), 2);
        assert!((column.group_ratios[0] - 0.5).abs() < 1e-4);
        assert!((column.group_ratios[1] - 0.5).abs() < 1e-4);
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn normalization_collapses_a_split_with_one_child_but_keeps_the_column() {
        let mut layout = initial_layout();
        let original_group = tab_group_id(&layout, 0, 0);
        let split = layout.insert_node(Node::Split(Split {
            axis: Axis::Vertical,
            children: vec![original_group],
            ratios: vec![1.0],
        }));
        layout.dock_areas[0].columns[0].groups[0] = split;

        assert!(layout.validate(&metrics()).is_err());
        layout
            .normalize(&metrics())
            .expect("single-child split is collapsed");
        assert_eq!(layout.dock_areas()[0].columns[0].groups[0], original_group);
        assert!(layout.nodes().get(split).is_none());
        assert_eq!(layout.dock_areas()[0].columns[0].groups.len(), 2);
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn normalization_removes_empty_columns_and_areas() {
        let mut layout = initial_layout();
        layout.dock_areas[0].columns.push(Column {
            width: 280.0,
            mode: ColumnMode::Expanded,
            groups: Vec::new(),
            group_ratios: Vec::new(),
        });
        layout.dock_areas.push(DockArea {
            side: DockSide::Left,
            columns: Vec::new(),
        });

        layout
            .normalize(&metrics())
            .expect("empty columns and areas are removed");
        assert_eq!(layout.dock_areas().len(), 1);
        assert_eq!(layout.dock_areas()[0].columns.len(), 1);
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn normalization_repairs_mismatched_or_unusable_ratios() {
        let mut layout = initial_layout();
        layout.dock_areas[0].columns[0].group_ratios = vec![2.0, 2.0];
        layout.normalize(&metrics()).expect("ratios are normalized");
        let ratios = &layout.dock_areas()[0].columns[0].group_ratios;
        assert!((ratios[0] - 0.5).abs() < 1e-4);
        assert!((ratios[1] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn validation_rejects_horizontal_photoshop_split_and_bad_floating_bounds() {
        let mut layout = initial_layout();
        let floating_panel = layout.panels.keys().next().expect("initial panel exists");
        layout.floating_groups.insert(FloatingGroup {
            panels: vec![floating_panel],
            active_index: 0,
            bounds: Rect::new(Point::new(10.0, 10.0), Size::new(0.0, 50.0)),
            z_order: 0,
        });
        let split_children = vec![tab_group_id(&layout, 0, 0), tab_group_id(&layout, 0, 1)];
        let horizontal = layout.insert_node(Node::Split(Split {
            axis: Axis::Horizontal,
            children: split_children,
            ratios: vec![0.5, 0.5],
        }));
        // Keep the split unreachable so the validator also verifies all arena entries.
        let _ = horizontal;

        let errors = layout
            .validate(&metrics())
            .expect_err("invalid floating bounds and placements fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::InvalidFloatingBounds(_)))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::PanelPlacedMoreThanOnce(_)))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::UnreachableNode(_)))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::PhotoshopSplitMustBeVertical(_)))
        );
    }

    #[test]
    fn singleton_panel_keys_cannot_be_duplicated() {
        let mut layout = initial_layout();
        let duplicate = layout.insert_panel(PanelInstance::new(
            PanelKey::new("layers").expect("valid panel key"),
            PanelConstraints::new(
                Size::new(100.0, 50.0),
                Size::new(160.0, 80.0),
                Size::new(300.0, 200.0),
                true,
            ),
        ));
        let first_group = tab_group_id(&layout, 0, 0);
        if let Some(Node::Tabs(tabs)) = layout.nodes.get_mut(first_group) {
            tabs.panels.push(duplicate);
        }

        let errors = layout
            .validate(&metrics())
            .expect_err("singleton key must be unique");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::SingletonPanelDuplicated(key) if key == "layers"
        )));
    }

    #[test]
    fn validation_rejects_missing_node_references() {
        let mut layout = initial_layout();
        let missing = tab_group_id(&layout, 0, 0);
        layout.nodes.remove(missing);

        let errors = layout
            .validate(&metrics())
            .expect_err("missing node reference must fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::NodeReferenceMissing(node) if *node == missing
        )));
    }

    #[test]
    fn validation_rejects_ratios_that_do_not_sum_to_one() {
        let mut layout = initial_layout();
        layout.dock_areas[0].columns[0].group_ratios = vec![0.25, 0.25];

        let errors = layout
            .validate(&metrics())
            .expect_err("invalid ratios must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::ColumnRatiosDoNotSumToOne { .. }))
        );
    }

    #[test]
    fn validation_rejects_a_panel_placed_in_dock_and_floating_group() {
        let mut layout = initial_layout();
        let panel_id = layout
            .panels
            .keys()
            .next()
            .expect("initial layout has panels");
        layout.insert_floating_group(FloatingGroup {
            panels: vec![panel_id],
            active_index: 0,
            bounds: Rect::new(Point::new(20.0, 20.0), Size::new(240.0, 180.0)),
            z_order: 1,
        });

        let errors = layout
            .validate(&metrics())
            .expect_err("duplicate placement must fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::PanelPlacedMoreThanOnce(panel) if *panel == panel_id
        )));
    }

    #[test]
    fn initial_layout_rejects_invalid_metrics() {
        let mut invalid = metrics();
        invalid.default_column_width = f32::NAN;
        assert!(Layout::initial_photoshop(&invalid).is_err());
    }

    #[test]
    fn activate_tab_updates_active_index_and_validates() {
        use crate::DockCommand;
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let outcome = layout
            .apply(DockCommand::ActivateTab { group, index: 1 }, &metrics())
            .expect("valid tab activation");
        let Node::Tabs(tabs) = layout.nodes().get(group).expect("group exists") else {
            panic!("group must be tabs");
        };
        assert_eq!(tabs.active_index, 1);
        assert_eq!(outcome.version, layout.version());
        assert!(outcome.changed_nodes.contains(&group));
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn close_and_reopen_panel_restores_a_valid_layout() {
        use crate::DockCommand;
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = layout
            .nodes()
            .get(group)
            .and_then(|node| match node {
                Node::Tabs(tabs) => tabs.panels.first().copied(),
                Node::Split(_) => None,
            })
            .expect("first panel exists");
        let instance = layout.panels().get(panel).expect("panel exists").clone();
        layout
            .apply(DockCommand::ClosePanel { panel }, &metrics())
            .expect("close panel");
        assert!(!layout.panels().contains_key(panel));
        assert!(layout.validate(&metrics()).is_ok());

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: instance.key().clone(),
                    constraints: instance.constraints().clone(),
                    target: None,
                },
                &metrics(),
            )
            .expect("reopen panel");
        assert!(
            layout
                .panels()
                .values()
                .any(|candidate| candidate.key() == instance.key())
        );
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn singleton_open_activates_existing_panel_without_duplicate() {
        use crate::DockCommand;
        let mut layout = initial_layout();
        let before = layout.panels().len();
        let panel = layout
            .panels()
            .iter()
            .find(|(_, panel)| panel.key().as_str() == "layers")
            .map(|(id, _)| id)
            .expect("layers exists");
        let instance = layout.panels().get(panel).expect("panel exists").clone();
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: instance.key().clone(),
                    constraints: instance.constraints().clone(),
                    target: None,
                },
                &metrics(),
            )
            .expect("reopening singleton activates it");
        assert_eq!(layout.panels().len(), before);
        assert!(layout.validate(&metrics()).is_ok());
    }

    #[test]
    fn split_top_creates_a_new_group_before_target_in_same_column() {
        use crate::{DockCommand, SplitSide};

        let mut layout = initial_layout();
        let metrics = metrics();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("target group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let original_canvas = *layout.canvas();

        let outcome = layout
            .apply(
                DockCommand::SplitGroup {
                    panel,
                    group,
                    side: SplitSide::Top,
                },
                &metrics,
            )
            .expect("top split should succeed");

        let area = &layout.dock_areas()[0];
        assert_eq!(area.columns.len(), 1);
        assert_eq!(area.columns[0].groups.len(), 3);
        let new_group = area.columns[0].groups[0];
        let Node::Tabs(new_tabs) = layout.nodes().get(new_group).expect("new group exists") else {
            panic!("new group must be Tabs");
        };
        assert_eq!(new_tabs.panels, vec![panel]);
        let Node::Tabs(source_tabs) = layout.nodes().get(group).expect("source group remains")
        else {
            panic!("source group must be Tabs");
        };
        assert!(!source_tabs.panels.contains(&panel));
        assert!((area.columns[0].group_ratios.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        assert_eq!(layout.canvas(), &original_canvas);
        assert!(outcome.changed_nodes.contains(&group));
        assert!(outcome.changed_nodes.contains(&new_group));
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn split_bottom_creates_a_new_group_after_target() {
        use crate::{DockCommand, SplitSide};

        let mut layout = initial_layout();
        let metrics = metrics();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("target group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };

        layout
            .apply(
                DockCommand::SplitGroup {
                    panel,
                    group,
                    side: SplitSide::Bottom,
                },
                &metrics,
            )
            .expect("bottom split should succeed");

        let column = &layout.dock_areas()[0].columns[0];
        let new_group = column.groups[1];
        let Node::Tabs(tabs) = layout.nodes().get(new_group).expect("new group exists") else {
            panic!("new group must be Tabs");
        };
        assert_eq!(tabs.panels, vec![panel]);
        assert_eq!(column.groups.len(), 3);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn split_left_and_right_create_adjacent_columns_and_preserve_width_rules() {
        use crate::{DockCommand, SplitSide};

        for side in [SplitSide::Left, SplitSide::Right] {
            let mut layout = initial_layout();
            let metrics = metrics();
            let group = tab_group_id(&layout, 0, 0);
            let panel = match layout.nodes().get(group).expect("target group exists") {
                Node::Tabs(tabs) => tabs.panels[0],
                Node::Split(_) => panic!("Photoshop groups must be Tabs"),
            };

            layout
                .apply(DockCommand::SplitGroup { panel, group, side }, &metrics)
                .expect("lateral split should succeed");

            let columns = &layout.dock_areas()[0].columns;
            assert_eq!(columns.len(), 2);
            let new_index = if side == SplitSide::Left { 0 } else { 1 };
            let new_column = &columns[new_index];
            assert!(new_column.width >= metrics.min_column_width);
            assert!((new_column.group_ratios.iter().sum::<f32>() - 1.0).abs() < 1e-4);
            let new_group = new_column.groups[0];
            let Node::Tabs(tabs) = layout.nodes().get(new_group).expect("new group exists") else {
                panic!("new group must be Tabs");
            };
            assert_eq!(tabs.panels, vec![panel]);
            assert!(layout.validate(&metrics).is_ok());
        }
    }

    #[test]
    fn split_rejects_column_limit_without_changing_layout() {
        use crate::{CommandError, DockCommand, SplitSide};

        let mut layout = initial_layout();
        let mut metrics = metrics();
        metrics.max_columns = 1;
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("target group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let version_before = layout.version();
        let groups_before = layout.dock_areas()[0].columns[0].groups.clone();

        let error = layout
            .apply(
                DockCommand::SplitGroup {
                    panel,
                    group,
                    side: SplitSide::Left,
                },
                &metrics,
            )
            .expect_err("maximum column count must be respected");

        assert!(matches!(
            error,
            CommandError::ColumnLimitReached { maximum: 1 }
        ));
        assert_eq!(layout.version(), version_before);
        assert_eq!(layout.dock_areas()[0].columns.len(), 1);
        assert_eq!(layout.dock_areas()[0].columns[0].groups, groups_before);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn lateral_split_expands_new_column_to_panel_minimum_width() {
        use crate::{DockCommand, SplitSide};

        let mut layout = initial_layout();
        let metrics = metrics();
        let source_group = tab_group_id(&layout, 0, 0);
        let old_panel = match layout
            .nodes()
            .get(source_group)
            .expect("target group exists")
        {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let old_instance = layout
            .panels
            .remove(old_panel)
            .expect("panel instance exists");
        let wide_panel = layout.panels.insert(PanelInstance::new(
            old_instance.key().clone(),
            PanelConstraints::new(
                Size::new(420.0, 120.0),
                Size::new(480.0, 240.0),
                Size::new(800.0, 900.0),
                true,
            ),
        ));
        if let Some(Node::Tabs(tabs)) = layout.nodes.get_mut(source_group) {
            tabs.panels[0] = wide_panel;
        }

        layout
            .apply(
                DockCommand::SplitGroup {
                    panel: wide_panel,
                    group: source_group,
                    side: SplitSide::Right,
                },
                &metrics,
            )
            .expect("lateral split should honor panel minimum width");

        let new_column = &layout.dock_areas()[0].columns[1];
        assert!((new_column.width - 420.0).abs() < 1e-3);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn vertical_split_rejects_when_known_height_cannot_fit_group_minimums() {
        use crate::{CommandError, DockCommand, SplitSide};

        let mut layout = initial_layout();
        let metrics = metrics();
        layout.set_canvas(crate::CanvasState {
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(900.0, 200.0)),
        });
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("target group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let version_before = layout.version();
        let groups_before = layout.dock_areas()[0].columns[0].groups.clone();

        let error = layout
            .apply(
                DockCommand::SplitGroup {
                    panel,
                    group,
                    side: SplitSide::Top,
                },
                &metrics,
            )
            .expect_err("three groups need more height than the known viewport provides");

        assert!(
            matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
            if parameter == "column height")
        );
        assert_eq!(layout.version(), version_before);
        assert_eq!(layout.dock_areas()[0].columns[0].groups, groups_before);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn split_rejects_panel_that_does_not_belong_to_target_group() {
        use crate::{CommandError, DockCommand, SplitSide};

        let mut layout = initial_layout();
        let metrics = metrics();
        let group = tab_group_id(&layout, 0, 0);
        let other_group = tab_group_id(&layout, 0, 1);
        let panel = match layout
            .nodes()
            .get(other_group)
            .expect("second group exists")
        {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let version_before = layout.version();

        let error = layout
            .apply(
                DockCommand::SplitGroup {
                    panel,
                    group,
                    side: SplitSide::Top,
                },
                &metrics,
            )
            .expect_err("panel must belong to target group");

        assert!(
            matches!(error, CommandError::PanelNotInGroup { panel: error_panel, group: error_group }
            if error_panel == panel && error_group == group)
        );
        assert_eq!(layout.version(), version_before);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn vertical_split_of_a_single_panel_group_normalizes_without_empty_groups() {
        use crate::{DockCommand, SplitSide};

        let mut layout = initial_layout();
        let metrics = metrics();
        let group = tab_group_id(&layout, 0, 0);
        let (panel_to_split, panel_to_close) =
            match layout.nodes().get(group).expect("target group exists") {
                Node::Tabs(tabs) => (tabs.panels[0], tabs.panels[1]),
                Node::Split(_) => panic!("Photoshop groups must be Tabs"),
            };
        layout
            .apply(
                DockCommand::ClosePanel {
                    panel: panel_to_close,
                },
                &metrics,
            )
            .expect("leave one panel in the source group");
        let refreshed_group = tab_group_id(&layout, 0, 0);

        layout
            .apply(
                DockCommand::SplitGroup {
                    panel: panel_to_split,
                    group: refreshed_group,
                    side: SplitSide::Top,
                },
                &metrics,
            )
            .expect("splitting the only panel must normalize safely");

        assert_eq!(layout.dock_areas()[0].columns[0].groups.len(), 2);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn resize_groups_clamps_both_directions_and_reports_actual_delta() {
        use crate::DockCommand;
        use crate::command::ColumnRef;

        let metrics = metrics();
        for (requested_delta, expected_delta) in [(500.0_f32, 170.0_f32), (-500.0, -170.0)] {
            let mut layout = initial_layout();
            layout.set_canvas(CanvasState {
                bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, 500.0)),
            });
            let column_ref = ColumnRef::new(DockSide::Right, 0);
            let outcome = layout
                .apply(
                    DockCommand::ResizeGroups {
                        column: column_ref,
                        boundary: 0,
                        delta: requested_delta,
                    },
                    &metrics,
                )
                .expect("group boundary resize should clamp to the available minimums");

            assert!((outcome.actual_value.expect("resize result") - expected_delta).abs() < 1e-3);
            let column = &layout.dock_areas()[0].columns[0];
            let first_height = column.group_ratios[0] * 500.0;
            let second_height = column.group_ratios[1] * 500.0;
            assert!(first_height >= metrics.min_group_height - 1e-3);
            assert!(second_height >= metrics.min_group_height - 1e-3);
            assert!((column.group_ratios.iter().sum::<f32>() - 1.0).abs() < 1e-4);
            assert!(layout.validate(&metrics).is_ok());

            // A clamped drag is a successful command, not an error.
            assert!(outcome.actual_value.is_some());
        }
    }

    #[test]
    fn resize_groups_rejects_non_finite_delta_bad_boundary_and_unknown_geometry_atomically() {
        use crate::command::ColumnRef;
        use crate::{CommandError, DockCommand};

        let metrics = metrics();
        let column_ref = ColumnRef::new(DockSide::Right, 0);
        let mut layout = initial_layout();
        layout.set_canvas(CanvasState {
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, 500.0)),
        });
        let ratios_before = layout.dock_areas()[0].columns[0].group_ratios.clone();
        let version_before = layout.version();

        for delta in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = layout
                .apply(
                    DockCommand::ResizeGroups {
                        column: column_ref,
                        boundary: 0,
                        delta,
                    },
                    &metrics,
                )
                .expect_err("non-finite delta must be rejected");
            assert!(
                matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
                if parameter == "delta")
            );
            assert_eq!(layout.version(), version_before);
            assert_eq!(
                layout.dock_areas()[0].columns[0].group_ratios,
                ratios_before
            );
            assert!(layout.validate(&metrics).is_ok());
        }

        let error = layout
            .apply(
                DockCommand::ResizeGroups {
                    column: column_ref,
                    boundary: 7,
                    delta: 10.0,
                },
                &metrics,
            )
            .expect_err("invalid boundary must be rejected");
        assert!(
            matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
            if parameter == "group boundary")
        );
        assert_eq!(layout.version(), version_before);
        assert_eq!(
            layout.dock_areas()[0].columns[0].group_ratios,
            ratios_before
        );

        let mut without_geometry = initial_layout();
        let error = without_geometry
            .apply(
                DockCommand::ResizeGroups {
                    column: column_ref,
                    boundary: 0,
                    delta: 10.0,
                },
                &metrics,
            )
            .expect_err("pixel delta requires known canvas height");
        assert!(
            matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
            if parameter == "canvas height")
        );
        assert_eq!(without_geometry.version(), 0);
        assert!(without_geometry.validate(&metrics).is_ok());
    }

    #[test]
    fn resize_column_clamps_to_minimum_and_reports_resulting_width() {
        use crate::DockCommand;
        use crate::command::ColumnRef;

        let metrics = metrics();
        let mut layout = initial_layout();
        let column_ref = ColumnRef::new(DockSide::Right, 0);
        let outcome = layout
            .apply(
                DockCommand::ResizeColumn {
                    column: column_ref,
                    width: 120.0,
                },
                &metrics,
            )
            .expect("an undersized request clamps to the column minimum");

        assert_eq!(outcome.actual_value, Some(metrics.min_column_width));
        assert_eq!(
            layout.dock_areas()[0].columns[0].width,
            metrics.min_column_width
        );
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn resize_column_preserves_minimum_document_width_when_geometry_is_known() {
        use crate::DockCommand;
        use crate::command::ColumnRef;

        let metrics = metrics();
        let mut layout = initial_layout();
        layout.set_canvas(CanvasState {
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(500.0, 800.0)),
        });

        let outcome = layout
            .apply(
                DockCommand::ResizeColumn {
                    column: ColumnRef::new(DockSide::Right, 0),
                    width: 900.0,
                },
                &metrics,
            )
            .expect("column width should clamp to leave the minimum document width");

        let column_width = layout.dock_areas()[0].columns[0].width;
        assert!((column_width - 460.0).abs() < 1e-3);
        assert_eq!(outcome.actual_value, Some(column_width));
        assert!(column_width >= metrics.min_column_width);
        assert!((500.0 - (column_width - 280.0) - metrics.min_document_width).abs() < 1e-3);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn resize_column_rejects_non_finite_and_negative_widths_atomically() {
        use crate::command::ColumnRef;
        use crate::{CommandError, DockCommand};

        let metrics = metrics();
        let mut layout = initial_layout();
        let column_ref = ColumnRef::new(DockSide::Right, 0);
        let old_width = layout.dock_areas()[0].columns[0].width;
        for width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
            let version_before = layout.version();
            let error = layout
                .apply(
                    DockCommand::ResizeColumn {
                        column: column_ref,
                        width,
                    },
                    &metrics,
                )
                .expect_err("invalid width must be rejected");
            assert!(
                matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
                if parameter == "width")
            );
            assert_eq!(layout.version(), version_before);
            assert_eq!(layout.dock_areas()[0].columns[0].width, old_width);
            assert!(layout.validate(&metrics).is_ok());
        }
    }

    fn layout_with_two_columns(metrics: &DockMetrics) -> Layout {
        use crate::{DockCommand, SplitSide};

        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("first group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop dock groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::SplitGroup {
                    panel,
                    group,
                    side: SplitSide::Right,
                },
                metrics,
            )
            .expect("split should create a neighboring column");
        assert_eq!(layout.dock_areas()[0].columns.len(), 2);
        layout
    }

    #[test]
    fn resize_column_boundary_clamps_pair_atomically_and_preserves_total_width() {
        use crate::DockCommand;
        use crate::command::ColumnRef;

        let metrics = metrics();
        for (requested_delta, expected_delta) in [(500.0_f32, 80.0_f32), (-500.0, -80.0)] {
            let mut layout = layout_with_two_columns(&metrics);
            let first_ref = ColumnRef::new(DockSide::Right, 0);
            let second_ref = ColumnRef::new(DockSide::Right, 1);
            let before = &layout.dock_areas()[0].columns;
            let total_before = before[0].width + before[1].width;
            let outcome = layout
                .apply(
                    DockCommand::ResizeColumnBoundary {
                        first: first_ref,
                        second: second_ref,
                        delta: requested_delta,
                    },
                    &metrics,
                )
                .expect("adjacent column resize should clamp to minimum widths");

            let columns = &layout.dock_areas()[0].columns;
            assert!((outcome.actual_value.expect("resize result") - expected_delta).abs() < 1e-3);
            assert!((columns[0].width - (280.0 + expected_delta)).abs() < 1e-3);
            assert!((columns[1].width - (280.0 - expected_delta)).abs() < 1e-3);
            assert!((columns[0].width + columns[1].width - total_before).abs() < 1e-3);
            assert!(
                columns
                    .iter()
                    .all(|column| column.width >= metrics.min_column_width)
            );
            assert!(layout.validate(&metrics).is_ok());
        }
    }

    #[test]
    fn resize_column_boundary_rejects_non_adjacent_or_reversed_columns_atomically() {
        use crate::command::ColumnRef;
        use crate::{CommandError, DockCommand, SplitSide};

        let metrics = metrics();
        let mut layout = initial_layout();
        for group_index in [0, 1] {
            let group = tab_group_id(&layout, 0, group_index);
            let panel = match layout.nodes().get(group).expect("source group exists") {
                Node::Tabs(tabs) => tabs.panels[0],
                Node::Split(_) => panic!("Photoshop dock groups must be Tabs"),
            };
            layout
                .apply(
                    DockCommand::SplitGroup {
                        panel,
                        group,
                        side: SplitSide::Right,
                    },
                    &metrics,
                )
                .expect("split should add a column while leaving the source group populated");
        }
        assert_eq!(layout.dock_areas()[0].columns.len(), 3);
        let widths_before: Vec<f32> = layout.dock_areas()[0]
            .columns
            .iter()
            .map(|column| column.width)
            .collect();
        let version_before = layout.version();

        for delta in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = layout
                .apply(
                    DockCommand::ResizeColumnBoundary {
                        first: ColumnRef::new(DockSide::Right, 0),
                        second: ColumnRef::new(DockSide::Right, 1),
                        delta,
                    },
                    &metrics,
                )
                .expect_err("non-finite boundary delta must be rejected");
            assert!(
                matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
                if parameter == "delta")
            );
            assert_eq!(layout.version(), version_before);
            assert_eq!(
                layout.dock_areas()[0]
                    .columns
                    .iter()
                    .map(|column| column.width)
                    .collect::<Vec<_>>(),
                widths_before
            );
            assert!(layout.validate(&metrics).is_ok());
        }

        for (first, second) in [
            (
                ColumnRef::new(DockSide::Right, 0),
                ColumnRef::new(DockSide::Right, 2),
            ),
            (
                ColumnRef::new(DockSide::Right, 1),
                ColumnRef::new(DockSide::Right, 0),
            ),
        ] {
            let error = layout
                .apply(
                    DockCommand::ResizeColumnBoundary {
                        first,
                        second,
                        delta: 10.0,
                    },
                    &metrics,
                )
                .expect_err("only the existing adjacent ordered pair may be resized");
            assert!(
                matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
                if parameter == "columns")
            );
            assert_eq!(layout.version(), version_before);
            assert_eq!(
                layout.dock_areas()[0]
                    .columns
                    .iter()
                    .map(|column| column.width)
                    .collect::<Vec<_>>(),
                widths_before
            );
            assert!(layout.validate(&metrics).is_ok());
        }
    }

    fn floating_rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    fn floating_id_for_panel(layout: &Layout, panel: crate::PanelId) -> crate::FloatingGroupId {
        layout
            .floating_groups()
            .iter()
            .find_map(|(id, group)| group.panels.contains(&panel).then_some(id))
            .expect("panel should be in a floating group")
    }

    #[test]
    fn float_panel_detaches_it_from_dock_and_preserves_canvas() {
        use crate::{DockCommand, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let canvas_before = *layout.canvas();

        let outcome = layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel),
                    rect: floating_rect(20.0, 30.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("valid panel should detach into a floating group");

        let floating_id = floating_id_for_panel(&layout, panel);
        let floating = layout
            .floating_groups()
            .get(floating_id)
            .expect("floating group exists");
        assert_eq!(floating.panels, vec![panel]);
        assert_eq!(floating.active_index, 0);
        assert_eq!(floating.bounds, floating_rect(20.0, 30.0, 300.0, 180.0));
        let Node::Tabs(source) = layout.nodes().get(group).expect("source group remains") else {
            panic!("source group must be Tabs");
        };
        assert!(!source.panels.contains(&panel));
        assert!(outcome.changed_nodes.contains(&group));
        assert_eq!(layout.canvas(), &canvas_before);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn float_group_detaches_all_tabs_and_normalizes_empty_column() {
        use crate::{DockCommand, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panels = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels.clone(),
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };

        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Group(group),
                    rect: floating_rect(-10.0, 15.0, 320.0, 200.0),
                },
                &metrics,
            )
            .expect("valid group should detach into one floating group");

        assert_eq!(layout.dock_areas()[0].columns[0].groups.len(), 1);
        let floating = layout
            .floating_groups()
            .values()
            .next()
            .expect("floating group exists");
        assert_eq!(floating.panels, panels);
        assert_eq!(floating.active_index, 0);
        assert!(
            layout.nodes().get(group).is_none(),
            "unreachable source node is normalized away"
        );
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn float_rejects_invalid_or_too_small_rect_without_mutating_layout() {
        use crate::{CommandError, DockCommand, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let panels_before = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels.clone(),
            Node::Split(_) => unreachable!(),
        };
        let version_before = layout.version();

        for rect in [
            floating_rect(0.0, 0.0, 0.0, 100.0),
            floating_rect(0.0, 0.0, f32::NAN, 200.0),
            floating_rect(0.0, 0.0, 80.0, 40.0),
        ] {
            let error = layout
                .apply(
                    DockCommand::Float {
                        source: FloatSource::Panel(panel),
                        rect,
                    },
                    &metrics,
                )
                .expect_err("invalid or undersized float bounds must be rejected");
            assert!(
                matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
                if parameter == "floating rect")
            );
            assert_eq!(layout.version(), version_before);
            assert_eq!(layout.floating_groups().len(), 0);
            let Node::Tabs(source) = layout.nodes().get(group).expect("source group remains")
            else {
                panic!("source group must remain Tabs");
            };
            assert_eq!(source.panels, panels_before);
            assert!(layout.validate(&metrics).is_ok());
        }
    }

    #[test]
    fn move_and_resize_floating_apply_the_requested_bounds() {
        use crate::DockCommand;

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: crate::FloatSource::Panel(panel),
                    rect: floating_rect(10.0, 20.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("initial float should succeed");
        let floating_id = floating_id_for_panel(&layout, panel);

        layout
            .apply(
                DockCommand::MoveFloating {
                    floating: floating_id,
                    rect: floating_rect(60.0, 70.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("floating group should move");
        let group_after_move = layout
            .floating_groups()
            .get(floating_id)
            .expect("floating exists");
        assert_eq!(
            group_after_move.bounds,
            floating_rect(60.0, 70.0, 300.0, 180.0)
        );

        layout
            .apply(
                DockCommand::ResizeFloating {
                    floating: floating_id,
                    rect: floating_rect(40.0, 50.0, 360.0, 240.0),
                },
                &metrics,
            )
            .expect("floating group should resize");
        let group_after_resize = layout
            .floating_groups()
            .get(floating_id)
            .expect("floating exists");
        assert_eq!(
            group_after_resize.bounds,
            floating_rect(40.0, 50.0, 360.0, 240.0)
        );
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn floating_rect_commands_reject_invalid_minimums_and_unknown_ids_atomically() {
        use crate::{CommandError, DockCommand};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: crate::FloatSource::Panel(panel),
                    rect: floating_rect(10.0, 20.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("initial float should succeed");
        let floating_id = floating_id_for_panel(&layout, panel);
        let before = layout
            .floating_groups()
            .get(floating_id)
            .expect("floating exists")
            .clone();
        let version_before = layout.version();

        let error = layout
            .apply(
                DockCommand::ResizeFloating {
                    floating: floating_id,
                    rect: floating_rect(10.0, 20.0, 100.0, 40.0),
                },
                &metrics,
            )
            .expect_err("minimum panel size must be respected");
        assert!(
            matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
            if parameter == "floating rect")
        );
        assert_eq!(layout.version(), version_before);
        assert_eq!(layout.floating_groups().get(floating_id), Some(&before));

        let error = layout
            .apply(
                DockCommand::MoveFloating {
                    floating: crate::FloatingGroupId::null(),
                    rect: floating_rect(20.0, 30.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect_err("unknown floating ID must be rejected");
        assert!(matches!(error, CommandError::UnknownFloatingGroup(_)));
        assert_eq!(layout.version(), version_before);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn raise_floating_moves_group_to_front_of_z_order() {
        use crate::DockCommand;

        let metrics = metrics();
        let mut layout = initial_layout();
        let first_group = tab_group_id(&layout, 0, 0);
        let second_group = tab_group_id(&layout, 0, 1);
        let panel_a = match layout.nodes().get(first_group).expect("first group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let panel_b = match layout
            .nodes()
            .get(second_group)
            .expect("second group exists")
        {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: crate::FloatSource::Panel(panel_a),
                    rect: floating_rect(0.0, 0.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("first floating group should be created");
        layout
            .apply(
                DockCommand::Float {
                    source: crate::FloatSource::Panel(panel_b),
                    rect: floating_rect(20.0, 20.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("second floating group should be created");
        let floating_a = floating_id_for_panel(&layout, panel_a);
        let floating_b = floating_id_for_panel(&layout, panel_b);
        let second_z = layout
            .floating_groups()
            .get(floating_b)
            .expect("second float exists")
            .z_order;

        layout
            .apply(
                DockCommand::RaiseFloating {
                    floating: floating_a,
                },
                &metrics,
            )
            .expect("floating group should be raised");

        assert!(
            layout
                .floating_groups()
                .get(floating_a)
                .expect("first float exists")
                .z_order
                > second_z
        );
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_four_sided_drops_create_groups_and_preserve_validation() {
        use crate::{DockCommand, DropTarget, DropZone};

        let metrics = metrics();
        for case in [
            "group-top",
            "group-bottom",
            "column-top",
            "column-bottom",
            "column-left",
            "column-right",
            "edge-left",
            "edge-right",
        ] {
            let mut layout = initial_layout();
            let source_group = tab_group_id(&layout, 0, 0);
            let target_group = tab_group_id(&layout, 0, 1);
            let moved_panel = match layout.nodes().get(source_group).unwrap() {
                Node::Tabs(tabs) => tabs.panels[1],
                Node::Split(_) => panic!("groups must be Tabs"),
            };
            let target = match case {
                "group-top" => DropTarget::Group {
                    group: target_group,
                    zone: DropZone::Top,
                },
                "group-bottom" => DropTarget::Group {
                    group: target_group,
                    zone: DropZone::Bottom,
                },
                "column-top" => DropTarget::Column {
                    column: crate::command::ColumnRef::new(DockSide::Right, 0),
                    zone: DropZone::Top,
                },
                "column-bottom" => DropTarget::Column {
                    column: crate::command::ColumnRef::new(DockSide::Right, 0),
                    zone: DropZone::Bottom,
                },
                "column-left" => DropTarget::Column {
                    column: crate::command::ColumnRef::new(DockSide::Right, 0),
                    zone: DropZone::Left,
                },
                "column-right" => DropTarget::Column {
                    column: crate::command::ColumnRef::new(DockSide::Right, 0),
                    zone: DropZone::Right,
                },
                "edge-left" => DropTarget::DockEdge {
                    side: DockSide::Left,
                },
                "edge-right" => DropTarget::DockEdge {
                    side: DockSide::Right,
                },
                _ => unreachable!(),
            };

            layout
                .apply(
                    DockCommand::MovePanel {
                        panel: moved_panel,
                        target,
                    },
                    &metrics,
                )
                .unwrap_or_else(|error| panic!("{case} drop should succeed: {error}"));

            let panel_group = layout
                .dock_areas()
                .iter()
                .flat_map(|area| area.columns.iter())
                .flat_map(|column| column.groups.iter().copied())
                .find(|group| {
                    matches!(
                        layout.nodes().get(*group),
                        Some(Node::Tabs(tabs)) if tabs.panels == vec![moved_panel]
                    )
                })
                .unwrap_or_else(|| panic!("{case} drop should create a one-panel group"));
            let placement_count = layout
                .dock_areas()
                .iter()
                .flat_map(|area| area.columns.iter())
                .flat_map(|column| column.groups.iter())
                .filter_map(|group| match layout.nodes().get(*group) {
                    Some(Node::Tabs(tabs)) => Some(
                        tabs.panels
                            .iter()
                            .filter(|panel| **panel == moved_panel)
                            .count(),
                    ),
                    _ => None,
                })
                .sum::<usize>()
                + layout
                    .floating_groups()
                    .values()
                    .map(|floating| {
                        floating
                            .panels
                            .iter()
                            .filter(|panel| **panel == moved_panel)
                            .count()
                    })
                    .sum::<usize>();
            assert_eq!(placement_count, 1, "{case} must not duplicate or lose the panel");
            assert!(layout.validate(&metrics).is_ok(), "{case} must leave a valid layout");

            match case {
                "group-top" | "group-bottom" => {
                    let column = &layout.dock_areas()[0].columns[0];
                    let new_index = column.groups.iter().position(|group| *group == panel_group).unwrap();
                    let target_index = column.groups.iter().position(|group| *group == target_group).unwrap();
                    if case == "group-top" {
                        assert!(new_index < target_index, "group-top inserts before the target group");
                    } else {
                        assert!(new_index > target_index, "group-bottom inserts after the target group");
                    }
                }
                "column-top" | "column-bottom" => {
                    let column = &layout.dock_areas()[0].columns[0];
                    if case == "column-top" {
                        assert_eq!(column.groups.first(), Some(&panel_group));
                    } else {
                        assert_eq!(column.groups.last(), Some(&panel_group));
                    }
                }
                "column-left" | "column-right" => {
                    let area = layout.dock_areas().iter().find(|area| area.side == DockSide::Right).unwrap();
                    let new_index = area.columns.iter().position(|column| column.groups.contains(&panel_group)).unwrap();
                    let target_index = area.columns.iter().position(|column| column.groups.contains(&target_group)).unwrap();
                    if case == "column-left" {
                        assert!(new_index < target_index);
                    } else {
                        assert!(new_index > target_index);
                    }
                }
                "edge-left" | "edge-right" => {
                    let side = if case == "edge-left" { DockSide::Left } else { DockSide::Right };
                    let area = layout.dock_areas().iter().find(|area| area.side == side).unwrap();
                    let new_index = area.columns.iter().position(|column| column.groups.contains(&panel_group)).unwrap();
                    if case == "edge-left" {
                        assert_eq!(new_index, 0);
                    } else {
                        assert_eq!(new_index, area.columns.len() - 1);
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn move_panel_side_drops_reject_impossible_minimums_atomically() {
        use crate::{CommandError, DockCommand, DropTarget, DropZone};

        let metrics = metrics();
        for (height, target_zone) in [
            (200.0, DropZone::Top),
            (246.0, DropZone::Bottom),
        ] {
            let mut layout = initial_layout();
            layout.set_canvas(CanvasState {
                bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, height)),
            });
            let source_group = tab_group_id(&layout, 0, 0);
            let panel = match layout.nodes().get(source_group).unwrap() {
                Node::Tabs(tabs) => tabs.panels[1],
                Node::Split(_) => panic!("groups must be Tabs"),
            };
            let version = layout.version();
            let err = layout.apply(
                DockCommand::MovePanel {
                    panel,
                    target: DropTarget::Column {
                        column: crate::command::ColumnRef::new(DockSide::Right, 0),
                        zone: target_zone,
                    },
                },
                &metrics,
            ).expect_err("a new group cannot fit alongside existing group minimums");
            assert!(matches!(err, CommandError::InvalidCommandParameter { .. }));
            assert_eq!(layout.version(), version, "rejected drop is atomic");
            assert!(layout.validate(&metrics).is_ok());
        }

        for zone in [DropZone::Top, DropZone::Bottom] {
            let mut layout = initial_layout();
            layout.set_canvas(CanvasState {
                bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, 200.0)),
            });
            let source_group = tab_group_id(&layout, 0, 0);
            let target_group = tab_group_id(&layout, 0, 1);
            let panel = match layout.nodes().get(source_group).unwrap() {
                Node::Tabs(tabs) => tabs.panels[1],
                Node::Split(_) => panic!("groups must be Tabs"),
            };
            let version = layout.version();
            let err = layout
                .apply(
                    DockCommand::MovePanel {
                        panel,
                        target: DropTarget::Group {
                            group: target_group,
                            zone,
                        },
                    },
                    &metrics,
                )
                .expect_err("group-side split must preserve all group minimum heights");
            assert!(matches!(err, CommandError::InvalidCommandParameter { .. }));
            assert_eq!(layout.version(), version);
            assert!(layout.validate(&metrics).is_ok());
        }

        let mut maxed_metrics = self::metrics();
        maxed_metrics.max_columns = 1;
        let mut maxed_layout = initial_layout();
        let maxed_source = tab_group_id(&maxed_layout, 0, 0);
        let maxed_panel = match maxed_layout.nodes().get(maxed_source).unwrap() {
            Node::Tabs(tabs) => tabs.panels[1],
            Node::Split(_) => panic!("groups must be Tabs"),
        };
        let maxed_version = maxed_layout.version();
        let maxed_error = maxed_layout
            .apply(
                DockCommand::MovePanel {
                    panel: maxed_panel,
                    target: DropTarget::Column {
                        column: crate::command::ColumnRef::new(DockSide::Right, 0),
                        zone: DropZone::Left,
                    },
                },
                &maxed_metrics,
            )
            .expect_err("maximum column count must be respected for side drops");
        assert!(matches!(
            maxed_error,
            CommandError::ColumnLimitReached { maximum: 1 }
        ));
        assert_eq!(maxed_layout.version(), maxed_version);
        assert!(maxed_layout.validate(&maxed_metrics).is_ok());

        let mut layout = initial_layout();
        layout.set_canvas(CanvasState {
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(450.0, 800.0)),
        });
        let source_group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(source_group).unwrap() {
            Node::Tabs(tabs) => tabs.panels[1],
            Node::Split(_) => panic!("groups must be Tabs"),
        };
        let version = layout.version();
        let err = layout.apply(
            DockCommand::MovePanel {
                panel,
                target: DropTarget::DockEdge {
                    side: DockSide::Left,
                },
            },
            &metrics,
        ).expect_err("new column must preserve the document minimum width");
        assert!(matches!(err, CommandError::InvalidCommandParameter { .. }));
        assert_eq!(layout.version(), version);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_center_drop_appends_and_activates_without_duplicate_instances() {
        use crate::{DockCommand, DropTarget, DropZone};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let target_group = tab_group_id(&layout, 0, 1);
        let moved_panel = match layout.nodes().get(source_group).unwrap() {
            Node::Tabs(tabs) => tabs.panels[1],
            Node::Split(_) => panic!("groups must be Tabs"),
        };
        let original_panel_count = layout.panels().len();

        layout
            .apply(
                DockCommand::MovePanel {
                    panel: moved_panel,
                    target: DropTarget::Group {
                        group: target_group,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect("center drop should add the panel as a tab");

        assert!(matches!(
            layout.nodes().get(source_group),
            Some(Node::Tabs(tabs)) if tabs.panels.len() == 1 && !tabs.panels.contains(&moved_panel)
        ));
        assert!(matches!(
            layout.nodes().get(target_group),
            Some(Node::Tabs(tabs))
                if tabs.panels.last() == Some(&moved_panel)
                    && tabs.active_index == tabs.panels.len() - 1
        ));
        let placed_count: usize = layout
            .dock_areas()
            .iter()
            .flat_map(|area| area.columns.iter())
            .flat_map(|column| column.groups.iter())
            .filter_map(|group| match layout.nodes().get(*group) {
                Some(Node::Tabs(tabs)) => Some(tabs.panels.len()),
                _ => None,
            })
            .sum();
        assert_eq!(layout.panels().len(), original_panel_count);
        assert_eq!(placed_count, original_panel_count);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_tab_insertion_places_and_activates_in_target_group() {
        use crate::{DockCommand, DropTarget};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let target_group = tab_group_id(&layout, 0, 1);
        let (moved_panel, remaining_panel) = match layout.nodes().get(source_group).unwrap() {
            Node::Tabs(tabs) => (tabs.panels[1], tabs.panels[0]),
            Node::Split(_) => panic!("groups must be Tabs"),
        };
        let original_panel_count = layout.panels().len();

        layout
            .apply(
                DockCommand::MovePanel {
                    panel: moved_panel,
                    target: DropTarget::TabInsertion {
                        group: target_group,
                        index: 1,
                    },
                },
                &metrics,
            )
            .expect("tab insertion should place the panel at the requested position");

        assert!(matches!(
            layout.nodes().get(source_group),
            Some(Node::Tabs(tabs)) if tabs.panels == vec![remaining_panel]
        ));
        assert!(matches!(
            layout.nodes().get(target_group),
            Some(Node::Tabs(tabs))
                if tabs.panels.len() == 3
                    && tabs.panels[1] == moved_panel
                    && tabs.active_index == 1
        ));
        let placed_count: usize = layout
            .dock_areas()
            .iter()
            .flat_map(|area| area.columns.iter())
            .flat_map(|column| column.groups.iter())
            .filter_map(|group| match layout.nodes().get(*group) {
                Some(Node::Tabs(tabs)) => Some(tabs.panels.len()),
                _ => None,
            })
            .sum();
        assert_eq!(layout.panels().len(), original_panel_count);
        assert_eq!(placed_count, original_panel_count);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_tab_insertion_reorders_within_group_without_loss_or_duplication() {
        use crate::DockCommand;

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let (moved_panel, other_panel) = match layout.nodes().get(group).unwrap() {
            Node::Tabs(tabs) => (tabs.panels[0], tabs.panels[1]),
            Node::Split(_) => panic!("groups must be Tabs"),
        };
        let original_panel_ids: Vec<_> = layout.panels().iter().map(|(id, _)| id).collect();
        let original_panel_count = layout.panels().len();

        layout
            .apply(
                DockCommand::MovePanel {
                    panel: moved_panel,
                    target: crate::DropTarget::TabInsertion { group, index: 2 },
                },
                &metrics,
            )
            .expect("same-group insertion should reorder the existing panel");

        assert!(matches!(
            layout.nodes().get(group),
            Some(Node::Tabs(tabs))
                if tabs.panels == vec![other_panel, moved_panel]
                    && tabs.active_index == 1
        ));
        let placed_panels: Vec<_> = layout
            .dock_areas()
            .iter()
            .flat_map(|area| area.columns.iter())
            .flat_map(|column| column.groups.iter())
            .filter_map(|group| match layout.nodes().get(*group) {
                Some(Node::Tabs(tabs)) => Some(tabs.panels.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        assert_eq!(layout.panels().len(), original_panel_count);
        assert_eq!(placed_panels.len(), original_panel_count);
        for panel in original_panel_ids {
            assert_eq!(placed_panels.iter().filter(|candidate| **candidate == panel).count(), 1);
        }
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_can_add_a_docked_tab_to_an_existing_floating_group() {
        use crate::{DockCommand, DropTarget, DropZone};

        let metrics = metrics();
        let mut layout = initial_layout();
        let first_group = tab_group_id(&layout, 0, 0);
        let second_group = tab_group_id(&layout, 0, 1);
        let target_panel = match layout.nodes().get(first_group).expect("first group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let moved_panel = match layout
            .nodes()
            .get(second_group)
            .expect("second group exists")
        {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: crate::FloatSource::Panel(target_panel),
                    rect: floating_rect(0.0, 0.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("target floating group should be created");
        let floating_id = floating_id_for_panel(&layout, target_panel);

        layout
            .apply(
                DockCommand::MovePanel {
                    panel: moved_panel,
                    target: DropTarget::FloatingGroup {
                        floating: floating_id,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect("panel should become a tab in the floating group");

        let floating = layout
            .floating_groups()
            .get(floating_id)
            .expect("floating group remains");
        assert_eq!(floating.panels, vec![target_panel, moved_panel]);
        assert_eq!(floating.active_index, 1);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn dock_floating_moves_all_tabs_to_dock_and_removes_floating_container() {
        use crate::{DockCommand, DropTarget, DropZone, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let target_group = tab_group_id(&layout, 0, 1);
        let source_panels = match layout
            .nodes()
            .get(source_group)
            .expect("source group exists")
        {
            Node::Tabs(tabs) => tabs.panels.clone(),
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let target_panels = match layout
            .nodes()
            .get(target_group)
            .expect("target group exists")
        {
            Node::Tabs(tabs) => tabs.panels.clone(),
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Group(source_group),
                    rect: floating_rect(0.0, 0.0, 320.0, 200.0),
                },
                &metrics,
            )
            .expect("source group should float");
        let floating_id = floating_id_for_panel(&layout, source_panels[0]);

        layout
            .apply(
                DockCommand::DockFloating {
                    floating: floating_id,
                    target: DropTarget::Group {
                        group: target_group,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect("floating group should dock as tabs into target group");

        assert!(layout.floating_groups().get(floating_id).is_none());
        let Node::Tabs(target) = layout
            .nodes()
            .get(target_group)
            .expect("target group remains")
        else {
            panic!("target group must be Tabs");
        };
        let mut expected_panels = target_panels;
        expected_panels.extend(source_panels.iter().copied());
        assert_eq!(target.panels, expected_panels);
        assert!(
            source_panels
                .iter()
                .all(|panel| target.panels.contains(panel))
        );
        assert_eq!(target.active_index, 2);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn dock_floating_split_zone_creates_a_docked_group_and_removes_source() {
        use crate::{DockCommand, DropTarget, DropZone, FloatSource, Node};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let target_group = tab_group_id(&layout, 0, 1);
        let source_panels = match layout.nodes().get(source_group).expect("source group exists") {
            Node::Tabs(tabs) => tabs.panels.clone(),
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Group(source_group),
                    rect: floating_rect(40.0, 40.0, 320.0, 220.0),
                },
                &metrics,
            )
            .expect("whole source group should float");
        let floating = floating_id_for_panel(&layout, source_panels[0]);

        layout
            .apply(
                DockCommand::DockFloating {
                    floating,
                    target: DropTarget::Group {
                        group: target_group,
                        zone: DropZone::Top,
                    },
                },
                &metrics,
            )
            .expect("floating group should dock through the target's top split zone");

        assert!(layout.floating_groups().get(floating).is_none());
        let target_location = layout
            .dock_areas()
            .iter()
            .enumerate()
            .find_map(|(area_index, area)| {
                area.columns.iter().enumerate().find_map(|(column_index, column)| {
                    column
                        .groups
                        .iter()
                        .position(|group| *group == target_group)
                        .map(|target_index| (area_index, column_index, target_index))
                })
            })
            .expect("target group remains docked");
        let (area_index, column_index, target_index) = target_location;
        assert!(target_index > 0, "top split inserts the new group before the target");
        let column = &layout.dock_areas()[area_index].columns[column_index];
        let new_group = column.groups[target_index - 1];
        let Node::Tabs(docked) = layout.nodes().get(new_group).unwrap() else {
            panic!("split result must be a tab group");
        };
        assert_eq!(docked.panels, source_panels);
        for panel in &source_panels {
            let occurrences = layout
                .dock_areas()
                .iter()
                .flat_map(|area| area.columns.iter())
                .flat_map(|column| column.groups.iter())
                .filter_map(|group| match layout.nodes().get(*group) {
                    Some(Node::Tabs(tabs)) => Some(tabs.panels.iter().filter(|id| *id == panel).count()),
                    Some(Node::Split(_)) | None => None,
                })
                .sum::<usize>();
            assert_eq!(occurrences, 1, "each original panel must remain placed exactly once");
        }
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn dock_floating_can_create_a_new_dock_edge_column() {
        use crate::{DockCommand, DockSide, DropTarget, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let panel = match layout
            .nodes()
            .get(source_group)
            .expect("source group exists")
        {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel),
                    rect: floating_rect(0.0, 0.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("panel should float");
        let floating_id = floating_id_for_panel(&layout, panel);

        layout
            .apply(
                DockCommand::DockFloating {
                    floating: floating_id,
                    target: DropTarget::DockEdge {
                        side: DockSide::Left,
                    },
                },
                &metrics,
            )
            .expect("floating group should dock at the left edge");

        assert_eq!(layout.dock_areas().len(), 2);
        assert_eq!(layout.dock_areas()[0].side, DockSide::Right);
        assert_eq!(layout.dock_areas()[1].side, DockSide::Left);
        assert!(layout.floating_groups().is_empty());
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_rejects_tab_that_exceeds_target_floating_minimum_size_atomically() {
        use crate::{
            CommandError, DockCommand, DropTarget, DropZone, PanelConstraints, PanelKey, Size,
        };

        let metrics = metrics();
        let mut layout = initial_layout();
        let dock_group = tab_group_id(&layout, 0, 0);
        let target_panel = match layout.nodes().get(dock_group).expect("dock group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: crate::FloatSource::Panel(target_panel),
                    rect: floating_rect(0.0, 0.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("target floating group should be created");
        let floating_id = floating_id_for_panel(&layout, target_panel);

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: PanelKey::new("wide-tool").expect("valid panel key"),
                    constraints: PanelConstraints::new(
                        Size::new(420.0, 100.0),
                        Size::new(480.0, 160.0),
                        Size::new(800.0, 600.0),
                        false,
                    ),
                    target: Some(DropTarget::Group {
                        group: dock_group,
                        zone: DropZone::Center,
                    }),
                },
                &metrics,
            )
            .expect("wide panel should open in the dock group");
        let wide_panel = layout
            .panels()
            .iter()
            .find_map(|(id, panel)| (panel.key().as_str() == "wide-tool").then_some(id))
            .expect("wide panel instance exists");
        let floating_before = layout
            .floating_groups()
            .get(floating_id)
            .expect("floating exists")
            .clone();
        let version_before = layout.version();

        let error = layout
            .apply(
                DockCommand::MovePanel {
                    panel: wide_panel,
                    target: DropTarget::FloatingGroup {
                        floating: floating_id,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect_err("target floating bounds cannot satisfy the incoming panel minimum width");
        assert!(
            matches!(error, CommandError::InvalidCommandParameter { ref parameter, .. }
            if parameter == "floating rect")
        );
        assert_eq!(layout.version(), version_before);
        assert_eq!(
            layout.floating_groups().get(floating_id),
            Some(&floating_before)
        );
        assert!(matches!(
            layout.nodes().get(dock_group),
            Some(Node::Tabs(tabs)) if tabs.panels.contains(&wide_panel)
        ));
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_transfers_a_tab_between_floating_groups_and_removes_empty_source() {
        use crate::{DockCommand, DropTarget, DropZone, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group_a = tab_group_id(&layout, 0, 0);
        let group_b = tab_group_id(&layout, 0, 1);
        let panel_a = match layout.nodes().get(group_a).expect("first group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let panel_b = match layout.nodes().get(group_b).expect("second group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let rect = floating_rect(0.0, 0.0, 300.0, 180.0);
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel_a),
                    rect,
                },
                &metrics,
            )
            .expect("first panel should float");
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel_b),
                    rect,
                },
                &metrics,
            )
            .expect("second panel should float");
        let target_id = floating_id_for_panel(&layout, panel_a);
        let source_id = floating_id_for_panel(&layout, panel_b);

        layout
            .apply(
                DockCommand::MovePanel {
                    panel: panel_b,
                    target: DropTarget::FloatingGroup {
                        floating: target_id,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect("panel should move to the other floating group's tabs");

        assert!(layout.floating_groups().get(source_id).is_none());
        let target = layout
            .floating_groups()
            .get(target_id)
            .expect("target remains");
        assert_eq!(target.panels, vec![panel_a, panel_b]);
        assert_eq!(target.active_index, 1);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn move_panel_from_floating_to_docked_group_removes_empty_source_without_duplicates() {
        use crate::{DockCommand, DropTarget, DropZone, FloatSource, Node};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let target_group = tab_group_id(&layout, 0, 1);
        let source_panel = match layout.nodes().get(source_group).expect("source group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let target_panel = match layout.nodes().get(target_group).expect("target group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let panel_count_before = layout.panels().len();

        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(source_panel),
                    rect: floating_rect(20.0, 24.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("source panel should float");
        let source_floating = floating_id_for_panel(&layout, source_panel);
        assert_eq!(
            layout.floating_groups().get(source_floating).unwrap().panels,
            vec![source_panel]
        );

        layout
            .apply(
                DockCommand::MovePanel {
                    panel: source_panel,
                    target: DropTarget::Group {
                        group: target_group,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect("floating panel should dock as a tab in the existing group");

        assert!(layout.floating_groups().get(source_floating).is_none());
        assert_eq!(layout.panels().len(), panel_count_before);
        let Node::Tabs(target) = layout.nodes().get(target_group).unwrap() else {
            panic!("target group must remain Tabs");
        };
        assert_eq!(target.panels.last(), Some(&source_panel));
        assert_eq!(target.active_index, target.panels.len() - 1);

        let mut occurrences = 0;
        for area in layout.dock_areas() {
            for column in &area.columns {
                for group in &column.groups {
                    if let Some(Node::Tabs(tabs)) = layout.nodes().get(*group) {
                        occurrences += tabs.panels.iter().filter(|id| **id == source_panel).count();
                    }
                }
            }
        }
        for floating in layout.floating_groups().values() {
            occurrences += floating.panels.iter().filter(|id| **id == source_panel).count();
        }
        assert_eq!(occurrences, 1, "the original PanelId must be placed exactly once");
        assert!(layout.panels().contains_key(target_panel));
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn failed_dock_of_floating_panel_preserves_its_source_and_layout_version() {
        use crate::{CommandError, DockCommand, DropTarget, FloatSource, Node};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let target_group = tab_group_id(&layout, 0, 1);
        let panel = match layout.nodes().get(source_group).expect("source group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel),
                    rect: floating_rect(20.0, 24.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("source panel should float");
        let floating = floating_id_for_panel(&layout, panel);
        let before = layout.floating_groups().get(floating).unwrap().clone();
        let version_before = layout.version();
        let target_before = match layout.nodes().get(target_group).unwrap() {
            Node::Tabs(tabs) => tabs.panels.clone(),
            Node::Split(_) => panic!("target group must be Tabs"),
        };

        let error = layout
            .apply(
                DockCommand::MovePanel {
                    panel,
                    target: DropTarget::TabInsertion {
                        group: target_group,
                        index: usize::MAX,
                    },
                },
                &metrics,
            )
            .expect_err("invalid target index should reject the transfer atomically");
        assert!(matches!(error, CommandError::InvalidTabIndex { .. }));
        assert_eq!(layout.version(), version_before);
        assert_eq!(layout.floating_groups().get(floating), Some(&before));
        assert_eq!(
            match layout.nodes().get(target_group).unwrap() {
                Node::Tabs(tabs) => tabs.panels.clone(),
                Node::Split(_) => panic!("target group must be Tabs"),
            },
            target_before
        );
        assert!(layout.panels().contains_key(panel));
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn docking_one_floating_group_into_another_merges_tabs_and_removes_source() {
        use crate::{DockCommand, DropTarget, DropZone, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let source_group = tab_group_id(&layout, 0, 0);
        let target_group = tab_group_id(&layout, 0, 1);
        let source_panel = match layout.nodes().get(source_group).expect("source group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let target_panel = match layout.nodes().get(target_group).expect("target group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        let count_before = layout.panels().len();
        let rect = floating_rect(20.0, 24.0, 320.0, 200.0);

        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(target_panel),
                    rect,
                },
                &metrics,
            )
            .expect("target panel should float");
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(source_panel),
                    rect: floating_rect(60.0, 64.0, 320.0, 200.0),
                },
                &metrics,
            )
            .expect("source panel should float");
        let target_floating = floating_id_for_panel(&layout, target_panel);
        let source_floating = floating_id_for_panel(&layout, source_panel);

        layout
            .apply(
                DockCommand::DockFloating {
                    floating: source_floating,
                    target: DropTarget::FloatingGroup {
                        floating: target_floating,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect("source floating group should merge into the target floating group");

        assert!(layout.floating_groups().get(source_floating).is_none());
        let target = layout.floating_groups().get(target_floating).unwrap();
        assert_eq!(target.panels, vec![target_panel, source_panel]);
        assert_eq!(target.active_index, 1);
        assert_eq!(layout.panels().len(), count_before);
        assert!(layout.panels().contains_key(source_panel));
        assert!(layout.panels().contains_key(target_panel));
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn dock_floating_rejects_free_area_target_without_removing_source() {
        use crate::{CommandError, DockCommand, DropTarget, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop groups must be Tabs"),
        };
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel),
                    rect: floating_rect(0.0, 0.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("panel should float");
        let floating_id = floating_id_for_panel(&layout, panel);
        let floating_before = layout
            .floating_groups()
            .get(floating_id)
            .expect("floating exists")
            .clone();
        let version_before = layout.version();

        let error = layout
            .apply(
                DockCommand::DockFloating {
                    floating: floating_id,
                    target: DropTarget::FreeArea {
                        bounds: floating_rect(10.0, 10.0, 320.0, 200.0),
                    },
                },
                &metrics,
            )
            .expect_err("free area is not a dock target");
        assert!(matches!(error, CommandError::InvalidDropTarget(_)));
        assert_eq!(layout.version(), version_before);
        assert_eq!(
            layout.floating_groups().get(floating_id),
            Some(&floating_before)
        );
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn activating_a_floating_tab_updates_the_existing_group_without_duplicating_panels() {
        use crate::{DockCommand, DropTarget, DropZone, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group_a = tab_group_id(&layout, 0, 0);
        let group_b = tab_group_id(&layout, 0, 1);
        let panel_a = match layout.nodes().get(group_a).expect("first group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop dock groups must be Tabs"),
        };
        let panel_b = match layout.nodes().get(group_b).expect("second group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop dock groups must be Tabs"),
        };
        let rect = floating_rect(0.0, 0.0, 300.0, 180.0);

        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel_a),
                    rect,
                },
                &metrics,
            )
            .expect("first panel should float");
        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel_b),
                    rect,
                },
                &metrics,
            )
            .expect("second panel should float");
        let floating = floating_id_for_panel(&layout, panel_a);

        layout
            .apply(
                DockCommand::MovePanel {
                    panel: panel_b,
                    target: DropTarget::FloatingGroup {
                        floating,
                        zone: DropZone::Center,
                    },
                },
                &metrics,
            )
            .expect("second panel should join the existing floating group");

        let count_before = layout.panels().len();
        layout
            .apply(
                DockCommand::ActivateFloatingTab { floating, index: 1 },
                &metrics,
            )
            .expect("existing floating tabs can be activated");

        let group = layout
            .floating_groups()
            .get(floating)
            .expect("floating group remains");
        assert_eq!(group.active_index, 1);
        assert_eq!(group.panels, vec![panel_a, panel_b]);
        assert_eq!(layout.panels().len(), count_before);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn closing_a_floating_panel_restores_its_last_dock_location() {
        use crate::{DockCommand, FloatSource};

        let metrics = metrics();
        let mut layout = initial_layout();
        let group = tab_group_id(&layout, 0, 0);
        let panel = match layout.nodes().get(group).expect("group exists") {
            Node::Tabs(tabs) => tabs.panels[0],
            Node::Split(_) => panic!("Photoshop dock groups must be Tabs"),
        };
        let instance = layout.panels().get(panel).expect("panel instance exists");
        let key = instance.key().to_owned();
        let constraints = (*instance.constraints()).clone();

        layout
            .apply(
                DockCommand::Float {
                    source: FloatSource::Panel(panel),
                    rect: floating_rect(0.0, 0.0, 300.0, 180.0),
                },
                &metrics,
            )
            .expect("panel should float");
        layout
            .apply(DockCommand::ClosePanel { panel }, &metrics)
            .expect("floating panel should close");

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: key.clone(),
                    constraints,
                    target: None,
                },
                &metrics,
            )
            .expect("closed panel should reopen");

        let reopened = layout
            .panels()
            .iter()
            .find_map(|(panel_id, instance)| (instance.key() == &key).then_some(panel_id))
            .expect("reopened panel instance exists");
        let restored_group = layout
            .nodes()
            .iter()
            .find_map(|(group_id, node)| match node {
                Node::Tabs(tabs) if tabs.panels.contains(&reopened) => Some(group_id),
                Node::Tabs(_) | Node::Split(_) => None,
            });
        assert_eq!(restored_group, Some(group));
        assert!(layout.validate(&metrics).is_ok());
    }
}
