use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Weak};

use slotmap::SlotMap;

use crate::{
    Axis, CanvasState, ChromeMode, Column, ColumnMode, CommandError, CommandOutcome, CommandValue,
    DockArea, DockAreaId, DockAreaKind, DockCommand, DockMetrics, DockSide, DropTarget, DropZone,
    FloatingGroup, FloatingGroupId, FlyoutState, GeometryError, LastDockLocation, Node, NodeId,
    Panel, PanelId, PanelInstance, PanelKey, Point, Profile, Rect, Size, Split, Tabs,
    ValidationError, command_application::CommandApplication,
};

const RATIO_TOLERANCE: f32 = 1.0e-4;
const PIXEL_TOLERANCE: f32 = 1.0e-3;
const MAX_REMEMBERED_DOCK_LOCATIONS: usize = 256;

/**
 * Internal validation memoization is not part of the logical Layout state.
 * Keep Debug snapshots stable when read-only operations warm the cache.
 */
#[derive(Clone, Default)]
struct ValidationCache(Cell<Option<(u64, DockMetrics)>>);

impl ValidationCache {
    fn get(&self) -> Option<(u64, DockMetrics)> {
        self.0.get()
    }

    fn set(&self, value: Option<(u64, DockMetrics)>) {
        self.0.set(value);
    }
}

impl fmt::Debug for ValidationCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ValidationCache")
    }
}

/// Arena parts restored from the persistent workspace DTO.
#[cfg(feature = "serde")]
pub(crate) struct WorkspaceLayoutParts {
    pub(crate) nodes: SlotMap<NodeId, Node>,
    pub(crate) panel_instances: SlotMap<PanelId, PanelInstance>,
    pub(crate) dock_areas: SlotMap<DockAreaId, DockArea>,
    pub(crate) main_areas: [Option<DockAreaId>; 2],
    pub(crate) floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    pub(crate) canvas: CanvasState,
    pub(crate) profile: Profile,
    pub(crate) chrome: ChromeMode,
}

#[cfg(feature = "serde")]
pub(crate) type FloatingBoundsClampResult =
    Result<Vec<(usize, Rect, Rect)>, (usize, crate::ComputeRectsError)>;

/// Complete UI-independent docking state.
///
/// Layout is the sole owner of runtime arenas. Public access is read-only;
/// external edits are performed only through Layout::apply and typed DockCommand
/// values. Clones retain the same history lineage; newly created or loaded layouts do not.
#[derive(Clone, Debug)]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panel_instances: SlotMap<PanelId, PanelInstance>,
    dock_areas: SlotMap<DockAreaId, DockArea>,
    main_areas: [Option<DockAreaId>; 2],
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    last_dock_locations: HashMap<PanelKey, LastDockLocation>,
    last_dock_location_order: VecDeque<PanelKey>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
    chrome: ChromeMode,
    flyout: Option<FlyoutState>,
    flyout_auto_collapse: bool,
    history_identity: Arc<()>,
    flyout_widths: HashMap<PanelKey, f32>,
    flyout_width_order: VecDeque<PanelKey>,
    validation_cache: ValidationCache,
}

mod column_display_order;
mod command_target_validation;
mod command_target_insertion;
mod column_resize;
mod floating_operations;
mod command_layout_state_dispatch;
mod command_flyout;
mod command_panels;

mod command_group_operations;
mod command_group_transfer;
mod command_state_comparison;
mod command_target_rail_insert;
mod drop_resolution;
mod commands;
mod ratio_helpers;
mod graph_validation_helpers;
mod accessors;
mod helpers;
mod history;
mod initialization;
mod node_normalization;
mod normalization_stages;
mod normalization;
mod workspace_column_width_repairs;
mod workspace_floating_bounds;
mod workspace_normalization;
mod workspace_photoshop_repairs;
mod workspace_singleton_repairs;
mod column_width_constraints;
mod group_sizing;
mod minimum_size;
mod main_column_width_reset;
mod tree_navigation;
mod tree_structure;
mod dock_location_memory;
#[cfg(test)]
mod tree;
mod validation_nodes;
mod validation_areas;
mod validation_graph;
mod validation_flyout;
mod validation_width;
mod validation;

#[cfg(test)]
mod tests;
