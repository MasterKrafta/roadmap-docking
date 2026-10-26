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
mod command_target_area_rejection_reason;
mod command_target_area_column;
mod command_target_area_group;
mod command_target_area_for_target;
mod command_target_validate_panel_target;
mod command_target_validate_panel_area;
mod command_target_validate_constraints_target;
mod command_target_group_icon_rail;
mod command_target_validate_drop;
mod command_floating_dock_operation;
mod command_floating_dock_main_column;
mod command_floating_dock_rail_slot;
mod command_floating_dock_column_slot;
mod command_floating_dock_tabs;
mod command_floating_dock_target;
mod floating_z_order;
mod command_floating_move;
mod command_floating_resize;
mod command_floating_resize_bounds;
mod command_floating_dispatch;
mod command_floating_window_dispatch;
mod command_floating_dock_dispatch;
mod command_floating_root_dispatch;
mod command_floating_group_column;
mod command_floating_panel_group;
mod command_group_resize_dispatch;
mod command_group_resize_boundary;
mod command_group_resize_reset;
mod command_column_resize_dispatch;
mod command_column_resize_reset;
mod command_column_resize_boundaries;
mod command_column_resize_single;
mod column_resize_helpers;
mod command_column_effective_width;
mod command_layout_state_dispatch;
mod command_flyout_dispatch;
mod command_panel_dispatch;
mod command_panel_open_dispatch;
mod command_panel_open_target;
mod command_panel_close_dispatch;
mod command_panel_tab_commands;
mod command_panel_open_new;
mod command_panel_reopen_singleton;
mod command_panel_transfer_dispatch;
mod command_panel_move_dispatch;
mod command_panel_transfer_inverse;
mod command_panel_target_transfer;
mod command_panel_tab_reorder;
mod command_flyout_panel_transfer_dispatch;
mod command_group_dispatch;
mod command_group_transfer_dispatch;
mod command_group_transfer_apply;
mod command_group_transfer_operations;
mod command_group_transfer_reorder;
mod command_changed_node_ids;
mod command_persistent_state_eq;
mod command_full_state_eq;
mod command_group_insertion;
mod command_group_merge_tabs;
mod command_group_edge_floating;
mod command_group_edge_generic;
mod command_group_edge_photoshop_column;
mod command_target_rail_panel_insert;
mod command_target_insert_node;
mod command_target_group_node;
mod command_target_group_insert;
mod command_target_rail_slot_index;
mod command_target_column_insert;
mod commands;
mod drag_drop;
mod ratio_helpers;
mod graph_validation_helpers;
mod accessors;
mod helpers;
mod history;
mod initialization;
mod normalization;
mod workspace_normalization;
mod column_width_constraints;
mod group_sizing;
mod minimum_layout_size;
mod main_column_width_reset;
mod tree;
mod validation;

#[cfg(test)]
mod tests;
