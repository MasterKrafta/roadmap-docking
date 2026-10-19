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

    /// Validate all model invariants, collecting every detectable error.
    ///
    /// Dimensions and column widths are logical pixels. Ratios are compared using a
    /// tolerance of 1e-4; pixel-sized values use 1e-3 where approximate comparison is needed.
    /// Applies one command atomically. A candidate copy is normalized and validated
    /// before it replaces the live layout, so every rejected command leaves the
    /// original state and version untouched.
    pub fn apply(
        &mut self,
        command: crate::command::DockCommand,
        metrics: &DockMetrics,
    ) -> Result<crate::command::CommandOutcome, crate::command::CommandError> {
        use crate::command::{CommandError, CommandOutcome};

        tracing::debug!(version = self.version, command = ?command, "applying dock command");

        let mut candidate = self.clone();
        if let Err(error) = candidate.apply_command_inner(command, metrics) {
            tracing::warn!(version = self.version, error = %error, "dock command rejected");
            return Err(error);
        }

        if let Err(errors) = candidate.normalize(metrics) {
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

        candidate.version = self.version.checked_add(1).ok_or(CommandError::VersionOverflow)?;
        let outcome = CommandOutcome::new(Vec::new(), candidate.version);
        *self = candidate;
        Ok(outcome)
    }

    fn apply_command_inner(
        &mut self,
        command: crate::command::DockCommand,
        _metrics: &DockMetrics,
    ) -> Result<(), crate::command::CommandError> {
        use crate::command::CommandError;

        // Concrete handlers are implemented in R3.3-R3.6. Rejecting an
        // unimplemented operation is safer than reporting success without mutation.
        let name = match command {
            crate::command::DockCommand::OpenPanel { .. } => "OpenPanel",
            crate::command::DockCommand::ClosePanel { .. } => "ClosePanel",
            crate::command::DockCommand::CloseGroup { .. } => "CloseGroup",
            crate::command::DockCommand::ActivateTab { .. } => "ActivateTab",
            crate::command::DockCommand::ReorderTab { .. } => "ReorderTab",
            crate::command::DockCommand::MovePanel { .. } => "MovePanel",
            crate::command::DockCommand::SplitGroup { .. } => "SplitGroup",
            crate::command::DockCommand::ResizeGroups { .. } => "ResizeGroups",
            crate::command::DockCommand::ResizeColumn { .. } => "ResizeColumn",
            crate::command::DockCommand::ResizeColumnBoundary { .. } => "ResizeColumnBoundary",
            crate::command::DockCommand::ResetColumnRatios { .. } => "ResetColumnRatios",
            crate::command::DockCommand::Float { .. } => "Float",
            crate::command::DockCommand::MoveFloating { .. } => "MoveFloating",
            crate::command::DockCommand::ResizeFloating { .. } => "ResizeFloating",
            crate::command::DockCommand::RaiseFloating { .. } => "RaiseFloating",
            crate::command::DockCommand::DockFloating { .. } => "DockFloating",
        };
        Err(CommandError::UnsupportedCommand(name.to_owned()))
    }

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
                    if minimum_width.is_finite()
                        && column.width + PIXEL_TOLERANCE < minimum_width
                    {
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
                if !column.group_ratios.iter().all(|ratio| ratio.is_finite() && *ratio >= 0.0) {
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
                    if !split.ratios.iter().all(|ratio| ratio.is_finite() && *ratio >= 0.0) {
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
                        for (child, ratio) in split.children.iter().copied().zip(split.ratios.iter().copied()) {
                            if !removed_nodes.contains(&child) {
                                children.push(child);
                                ratios.push(ratio);
                            }
                        }
                        split.children = children;
                        split.ratios = ratios;
                    } else {
                        split.children.retain(|child| !removed_nodes.contains(child));
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
        let next = self.version.checked_add(1).ok_or("layout version exhausted")?;
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
    [min.width, min.height, preferred.width, preferred.height, max.width, max.height]
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
    *ratios = if ratios_were_aligned { new_ratios } else { old_ratios };
}

fn normalize_ratios(ratios: &mut Vec<f32>, expected_len: usize) {
    if expected_len == 0 {
        ratios.clear();
        return;
    }

    let sum: f32 = ratios.iter().sum();
    let usable = ratios.len() == expected_len
        && ratios.iter().all(|ratio| ratio.is_finite() && *ratio >= 0.0)
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
        Axis, CanvasState, Column, ColumnMode, DockArea, DockMetrics, DockSide, FloatingGroup, Node,
        PanelConstraints, PanelInstance, PanelKey, Point, Profile, Rect, Size, Split, Tabs,
        ValidationError,
    };

    fn metrics() -> DockMetrics {
        DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
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
        assert_eq!(layout.panels().get(layers_tabs.panels[0]).map(|panel| panel.key().as_str()), Some("layers"));
        assert_eq!(layout.panels().get(layers_tabs.panels[1]).map(|panel| panel.key().as_str()), Some("color"));

        assert_eq!(properties_tabs.panels.len(), 2);
        assert_eq!(properties_tabs.active_index, 0);
        assert_eq!(layout.panels().get(properties_tabs.panels[0]).map(|panel| panel.key().as_str()), Some("properties"));
        assert_eq!(layout.panels().get(properties_tabs.panels[1]).map(|panel| panel.key().as_str()), Some("history"));

        assert_eq!(layout.panels().len(), 4);
        assert!(layout.panels().values().all(|panel| panel.constraints().is_singleton()));
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

        let errors = layout.validate(&metrics()).expect_err("corrupt layout must fail");
        assert!(errors.iter().any(|error| matches!(error, ValidationError::InvalidColumnWidth { .. })));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::ColumnRatioCountMismatch { .. })));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::ActiveTabOutOfBounds(_))));
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

        let errors = layout.validate(&metrics()).expect_err("duplicate placement must fail");
        assert!(errors.iter().any(|error| matches!(error, ValidationError::PanelPlacedMoreThanOnce(_))));
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
        assert!(errors.iter().any(|error| matches!(error, ValidationError::NodeCycle(_))));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::SplitTooFewChildren(_))));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::PhotoshopColumnGroupMustBeTabs { .. })));
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
        layout.normalize(&metrics()).expect("normalization preserves a single-group column");
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

        let errors = layout.validate(&metrics()).expect_err("too many columns must fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::TooManyColumns { actual: 5, maximum: 4 }
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
        layout.normalize(&metrics()).expect("empty group is removed");
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
        layout.normalize(&metrics()).expect("single-child split is collapsed");
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

        layout.normalize(&metrics()).expect("empty columns and areas are removed");
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

        let errors = layout.validate(&metrics()).expect_err("invalid floating bounds and placements fail");
        assert!(errors.iter().any(|error| matches!(error, ValidationError::InvalidFloatingBounds(_))));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::PanelPlacedMoreThanOnce(_))));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::UnreachableNode(_))));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::PhotoshopSplitMustBeVertical(_))));
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

        let errors = layout.validate(&metrics()).expect_err("singleton key must be unique");
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

        let errors = layout.validate(&metrics()).expect_err("missing node reference must fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::NodeReferenceMissing(node) if *node == missing
        )));
    }

    #[test]
    fn validation_rejects_ratios_that_do_not_sum_to_one() {
        let mut layout = initial_layout();
        layout.dock_areas[0].columns[0].group_ratios = vec![0.25, 0.25];

        let errors = layout.validate(&metrics()).expect_err("invalid ratios must fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::ColumnRatiosDoNotSumToOne { .. }
        )));
    }

    #[test]
    fn validation_rejects_a_panel_placed_in_dock_and_floating_group() {
        let mut layout = initial_layout();
        let panel_id = layout.panels.keys().next().expect("initial layout has panels");
        layout.insert_floating_group(FloatingGroup {
            panels: vec![panel_id],
            active_index: 0,
            bounds: Rect::new(Point::new(20.0, 20.0), Size::new(240.0, 180.0)),
            z_order: 1,
        });

        let errors = layout.validate(&metrics()).expect_err("duplicate placement must fail");
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
}
