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
    flyout_auto_collapse: bool,
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
            flyout_auto_collapse: true,
        }
    }

    #[cfg(feature = "serde")]
    pub(crate) fn from_workspace_parts(
        nodes: SlotMap<NodeId, Node>,
        panel_instances: SlotMap<PanelId, PanelInstance>,
        dock_areas: SlotMap<DockAreaId, DockArea>,
        main_areas: [Option<DockAreaId>; 2],
        floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
        canvas: CanvasState,
        profile: Profile,
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
            version: 0,
            chrome,
            flyout: None,
            flyout_auto_collapse: true,
        }
    }

    #[cfg(feature = "serde")]
    pub(crate) fn clamp_floating_groups_for_workspace_load(
        &mut self,
        client_area: Rect,
        metrics: &DockMetrics,
    ) -> Result<Vec<(usize, Rect, Rect)>, (usize, crate::ComputeRectsError)> {
        let mut groups: Vec<_> = self
            .floating_groups
            .iter()
            .map(|(id, group)| (id, group.z_order))
            .collect();
        groups.sort_by_key(|(_, z_order)| *z_order);

        let mut changes = Vec::new();
        for (index, (id, _)) in groups.into_iter().enumerate() {
            let Some(group) = self.floating_groups.get(id).copied() else {
                return Err((index, crate::ComputeRectsError::InvalidLayout));
            };
            let Some(root) = self.dock_areas.get(group.area).map(|area| area.root) else {
                return Err((index, crate::ComputeRectsError::InvalidLayout));
            };
            let original = group.bounds;
            let clamped = crate::compute_rects::clamp_floating_bounds(
                self,
                root,
                original,
                client_area,
                metrics,
            )
            .map_err(|error| (index, error))?;

            if clamped != original {
                let Some(group) = self.floating_groups.get_mut(id) else {
                    return Err((index, crate::ComputeRectsError::InvalidLayout));
                };
                group.bounds = clamped;
                changes.push((index, original, clamped));
            }
        }
        Ok(changes)
    }

    #[cfg(feature = "serde")]
    pub(crate) fn normalize_for_workspace_load(
        &mut self,
        metrics: &DockMetrics,
    ) -> Result<Option<Vec<String>>, Vec<ValidationError>> {
        let original_issues = self.validate(metrics).err().unwrap_or_default();
        let mut candidate = self.clone();
        let mut repair_notes = Vec::new();

        // Column presentation mode uniquely determines rail width. Correcting
        // a stale width is deterministic; an invalid expanded width is repaired
        // only when it is finite and positive, since otherwise intent is unclear.
        for (node_id, node) in candidate.nodes.iter_mut() {
            let Node::Column(column) = node else {
                continue;
            };
            match column.mode {
                ColumnMode::IconsOnly if column.width != metrics.rail_width => {
                    repair_notes.push(format!(
                        "column {node_id:?}: IconsOnly width reset from {} to {}",
                        column.width, metrics.rail_width
                    ));
                    column.width = metrics.rail_width;
                }
                ColumnMode::IconsAndLabels if column.width != metrics.min_column_width => {
                    repair_notes.push(format!(
                        "column {node_id:?}: IconsAndLabels width reset from {} to {}",
                        column.width, metrics.min_column_width
                    ));
                    column.width = metrics.min_column_width;
                }
                ColumnMode::Expanded if column.width.is_finite() && column.width > 0.0 => {
                    if column.width < metrics.min_column_width {
                        repair_notes.push(format!(
                            "column {node_id:?}: expanded width raised from {} to minimum {}",
                            column.width, metrics.min_column_width
                        ));
                        column.width = metrics.min_column_width;
                    }
                    if column
                        .last_expanded_width
                        .is_none_or(|width| !width.is_finite() || width <= 0.0)
                    {
                        repair_notes.push(format!(
                            "column {node_id:?}: missing or invalid last_expanded_width restored from current width {}",
                            column.width
                        ));
                        column.last_expanded_width = Some(column.width);
                    }
                }
                _ => {}
            }
        }

        let mut diagnostic_issues: Vec<String> =
            original_issues.iter().map(ToString::to_string).collect();
        diagnostic_issues.extend(repair_notes);

        if original_issues.is_empty() && diagnostic_issues.is_empty() {
            return Ok(None);
        }
        candidate.normalize_in_place(metrics)?;
        *self = candidate;
        Ok(Some(diagnostic_issues))
    }

    /// Captures a session-only history checkpoint without transient flyout UI.
    pub(crate) fn history_snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.flyout = None;
        snapshot
    }

    /// Restores a previously validated history checkpoint as a new model revision.
    ///
    /// The model revision is monotonic even though the persistent state moves
    /// backward/forward. Transient flyout state is not restored, and the current
    /// flyout auto-collapse preference remains session-local.
    pub(crate) fn restore_history_snapshot(
        &mut self,
        snapshot: &Self,
        metrics: &DockMetrics,
    ) -> Result<(), CommandError> {
        let next_version = self.version.checked_add(1).ok_or(CommandError::InvalidValue {
            field: "version",
            reason: "layout version overflow",
        })?;

        let mut candidate = snapshot.clone();
        candidate.flyout = None;
        candidate.flyout_auto_collapse = self.flyout_auto_collapse;
        candidate.version = next_version;
        candidate.validate(metrics).map_err(|errors| CommandError::InvalidLayout { errors })?;
        *self = candidate;
        Ok(())
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

    /// Whether passive dismissal and clicking the selected rail icon close a flyout.
    pub const fn flyout_auto_collapse_enabled(&self) -> bool {
        self.flyout_auto_collapse
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
        let index = helpers::side_index(side);
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
}

mod commands;
mod column_display_order;
mod drag_drop;
mod tree;
mod sizing;
mod validation;
mod normalization;
mod helpers;

#[cfg(test)]
mod tests;
