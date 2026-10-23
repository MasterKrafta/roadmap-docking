mod columns;
mod floating;
mod initial_layout;
mod rendering;
mod visibility;

use dock_core::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, FloatingGroupId, Layout, Node, NodeId,
    PanelId, PanelKey,
};
use dock_gpui::dock_metrics_from_theme;
use ui::Theme;

use crate::panels::PanelRegistry;
use visibility::{DockTabContextMenu, PanelVisibilityEntry};

/// Runtime view data derived from one Core Tabs node.
struct TabGroupViewData {
    id: NodeId,
    panels: Vec<PanelId>,
    active_panel: Option<PanelId>,
    collapsed: bool,
}

#[derive(Clone)]
struct ClosedTabGroupSnapshot {
    panel_keys: Vec<String>,
    active_panel_key: Option<String>,
    collapsed: bool,
}

#[derive(Clone)]
struct ClosedColumnSnapshot {
    /// Visual insertion slot captured from Core before the column was closed.
    position: usize,
    groups: Vec<ClosedTabGroupSnapshot>,
    group_ratios: Vec<f32>,
    width: f32,
}

#[derive(Clone, Copy)]
struct GroupResizeState {
    column: NodeId,
    boundary_index: usize,
    available_height: f32,
    splitter_hit_height: f32,
    group_area_top: f32,
}

/// Main window rendering the dock Core layout and its interactive panel controls.
pub(crate) struct MainWindow {
    theme: Theme,
    panels: PanelRegistry,
    layout: Layout,
    dock_metrics: DockMetrics,
    /// Stable column slots; current visual order is derived from the Core Split.
    column_ids: Vec<NodeId>,
    /// True only while a slot's Core column is active in the main or floating tree.
    /// Closed slots remain allocated so their saved snapshots can be restored.
    column_open: Vec<bool>,
    /// Per-panel visibility entries used by rendering, context actions, and the Windows menu.
    panel_visibility: Vec<PanelVisibilityEntry>,
    /// Core topology remembered while a column is closed, so it can be recreated generically.
    closed_column_snapshots: Vec<Option<ClosedColumnSnapshot>>,
    windows_menu_open: bool,
    tab_context_menu: Option<DockTabContextMenu>,
    is_resizing_column: bool,
    resizing_column_boundary_index: usize,
    /// The outermost dock column whose canvas-facing edge is being resized.
    resizing_outer_column: Option<NodeId>,
    /// Cursor-to-divider offset captured on mouse-down; keeps the pointer anchored
    /// to the actual boundary even after Core clamps the requested resize.
    resize_column_pointer_offset_x: f32,
    resizing_groups_column: Option<GroupResizeState>,
    resize_group_pointer_offset_y: f32,
    /// The complete horizontal main-dock composition while it is floating.
    floating_composition: Option<FloatingGroupId>,
    /// Pointer state used to detach or move the composition from a column header spacer.
    composition_drag_pending: bool,
    composition_dragging: bool,
    composition_drag_start_x: f32,
    composition_drag_start_y: f32,
    composition_drag_offset_x: f32,
    composition_drag_offset_y: f32,
    composition_drag_source: Option<NodeId>,
    composition_drag_group: Option<FloatingGroupId>,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let panels = PanelRegistry::default();
        let dock_metrics = dock_metrics_from_theme(&theme);
        let split_ratio = theme.metrics.default_column_split_ratio;
        if !split_ratio.is_finite() || split_ratio <= 0.0 || split_ratio >= 1.0 {
            return Err("Theme default_column_split_ratio must be between zero and one".to_owned());
        }

        let (layout, column_ids) =
            initial_layout::create_initial_layout(&panels, &dock_metrics, split_ratio)?;
        let initial_column_count = column_ids.len();
        let mut main_window = Self {
            theme,
            panels,
            layout,
            dock_metrics,
            column_ids,
            column_open: vec![true; initial_column_count],
            panel_visibility: Vec::new(),
            closed_column_snapshots: (0..initial_column_count).map(|_| None).collect(),
            windows_menu_open: false,
            tab_context_menu: None,
            is_resizing_column: false,
            resizing_column_boundary_index: 0,
            resizing_outer_column: None,
            resize_column_pointer_offset_x: 0.0,
            resizing_groups_column: None,
            resize_group_pointer_offset_y: 0.0,
            floating_composition: None,
            composition_drag_pending: false,
            composition_dragging: false,
            composition_drag_start_x: 0.0,
            composition_drag_start_y: 0.0,
            composition_drag_offset_x: 0.0,
            composition_drag_offset_y: 0.0,
            composition_drag_source: None,
            composition_drag_group: None,
        };
        main_window.sync_panel_visibility_entries();
        Ok(main_window)
    }

    /// Apply a Core layout mutation and then reconcile floating composition geometry.
    ///
    /// Keeping this in one path means column mode changes, group removal, and resize
    /// commands all refresh the outer floating bounds from the actual child columns.
    fn apply_dock_command(&mut self, command: DockCommand) -> Result<(), String> {
        self.layout
            .apply(command, &self.dock_metrics)
            .map_err(|error| error.to_string())?;
        self.sync_column_slots_from_core();
        self.sync_floating_composition_bounds();
        self.sync_panel_visibility_entries();
        Ok(())
    }

    /// Reconcile stable UI slots with the columns that currently exist in Core.
    ///
    /// Core node IDs are authoritative for active membership, while stale slot IDs
    /// and snapshots are retained so closed columns can still be restored.
    fn sync_column_slots_from_core(&mut self) {
        if self
            .floating_composition
            .is_some_and(|group_id| self.layout.floating_group(group_id).is_none())
        {
            self.floating_composition = None;
            self.composition_drag_pending = false;
            self.composition_dragging = false;
            self.composition_drag_source = None;
            self.composition_drag_group = None;
        }

        let mut active_column_ids = Vec::new();
        if let Some(root) = self.layout.main_area(DockSide::Right).map(|area| area.root) {
            active_column_ids.extend(self.core_column_ids_in_order(root));
        }
        if let Some(root) = self.floating_composition.and_then(|group_id| {
            self.layout
                .floating_group(group_id)
                .and_then(|floating| self.layout.dock_area(floating.area))
                .map(|area| area.root)
        }) {
            active_column_ids.extend(self.core_column_ids_in_order(root));
        }

        for column_id in active_column_ids.iter().copied() {
            if self.column_ids.contains(&column_id) {
                continue;
            }
            self.column_ids.push(column_id);
            self.column_open.push(true);
            self.closed_column_snapshots.push(None);
        }

        for (index, column_id) in self.column_ids.iter().copied().enumerate() {
            self.column_open[index] = active_column_ids.contains(&column_id);
        }
    }

    fn activate_tab(&mut self, group: NodeId, panel: PanelId) {
        if let Err(error) = self.apply_dock_command(
            DockCommand::ActivateTab { group, panel }) {
            eprintln!("Could not activate dock tab: {error}");
        }
    }

    fn active_panel_id(&self, group: NodeId) -> Option<PanelId> {
        let Node::Tabs(tabs) = self.layout.node(group)? else {
            return None;
        };
        let active_node = tabs.active_child()?;
        match self.layout.node(active_node)? {
            Node::Panel(panel) => Some(panel.panel_id),
            _ => None,
        }
    }

    fn tab_group_view_data(&self, group_id: NodeId) -> Option<TabGroupViewData> {
        let Node::Tabs(tabs) = self.layout.node(group_id)? else {
            return None;
        };
        let panels = tabs.children.iter().filter_map(|child_id| {
            match self.layout.node(*child_id)? {
                Node::Panel(panel) => Some(panel.panel_id),
                _ => None,
            }
        }).collect();

        Some(TabGroupViewData {
            id: group_id,
            panels,
            active_panel: self.active_panel_id(group_id),
            collapsed: tabs.collapsed,
        })
    }

    fn group_is_collapsed(&self, group: NodeId) -> bool {
        matches!(self.layout.node(group), Some(Node::Tabs(tabs)) if tabs.collapsed)
    }

    fn toggle_group_collapse(&mut self, group: NodeId) {
        let Some(Node::Tabs(tabs)) = self.layout.node(group) else {
            return;
        };
        let collapsed = !tabs.collapsed;
        if let Err(error) = self.apply_dock_command(
            DockCommand::CollapseGroup { group, collapsed }) {
            eprintln!("Could not change dock group collapse state: {error}");
        }
    }

    fn toggle_column_mode(&mut self, column_index: usize) {
        if column_index >= self.column_ids.len() {
            return;
        }
        let mode = if matches!(
            self.layout.node(self.column_ids[column_index]),
            Some(Node::Column(column)) if column.mode == ColumnMode::IconsOnly
        ) {
            ColumnMode::Expanded
        } else {
            ColumnMode::IconsOnly
        };
        if let Err(error) = self.apply_dock_command(
            DockCommand::SetColumnMode {
                column: self.column_ids[column_index],
                mode,
            }) {
            eprintln!("Could not change dock column mode: {error}");
        }
    }

    fn group_ids_in_column(&self, column_id: NodeId) -> Vec<NodeId> {
        group_ids_for_column(&self.layout, column_id)
    }

    fn group_ratios_in_column(&self, column_id: NodeId) -> Vec<f32> {
        match self.layout.node(column_id) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => Vec::new(),
        }
    }

    fn core_column_ids_in_order(&self, root: NodeId) -> Vec<NodeId> {
        match self.layout.node(root) {
            Some(Node::Column(_)) => vec![root],
            Some(Node::Split(split)) if split.axis == Axis::X => split
                .children
                .iter()
                .copied()
                .filter(|child| matches!(self.layout.node(*child), Some(Node::Column(_))))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn column_indices_in_core_order(&self, root: NodeId) -> Vec<usize> {
        self.core_column_ids_in_order(root)
            .iter()
            .filter_map(|column_id| {
                self.column_ids
                    .iter()
                    .position(|slot_id| slot_id == column_id)
            })
            .collect()
    }

    fn main_column_indices_in_core_order(&self) -> Vec<usize> {
        self.layout
            .main_area(DockSide::Right)
            .map(|area| self.column_indices_in_core_order(area.root))
            .unwrap_or_default()
    }

    /// Return a column's visual position, without confusing it with its stable UI slot.
    fn column_visual_position(&self, column_index: usize) -> usize {
        if let Some(area) = self.layout.main_area(DockSide::Right) {
            let columns = self.column_indices_in_core_order(area.root);
            if let Some(position) = columns.iter().position(|index| *index == column_index) {
                return position;
            }
        }

        if let Some(root) = self.floating_composition.and_then(|group_id| {
            self.layout
                .floating_group(group_id)
                .and_then(|floating| self.layout.dock_area(floating.area))
                .map(|area| area.root)
        }) {
            let columns = self.composition_column_indices(root);
            if let Some(position) = columns.iter().position(|index| *index == column_index) {
                return position;
            }
        }

        column_index
    }

    fn column_label(&self, column_index: usize) -> String {
        match self.column_visual_position(column_index) {
            0 => "Left".to_owned(),
            1 => "Right".to_owned(),
            position => format!("Column {}", position + 1),
        }
    }

}

fn group_ids_for_column(layout: &Layout, column_id: NodeId) -> Vec<NodeId> {
    match layout.node(column_id) {
        Some(Node::Column(column)) => column.groups.clone(),
        _ => Vec::new(),
    }
}

fn panel_id_in_group(layout: &Layout, group_id: NodeId, key: &PanelKey) -> Option<PanelId> {
    let Node::Tabs(tabs) = layout.node(group_id)? else {
        return None;
    };
    tabs.children.iter().find_map(|node_id| {
        let Node::Panel(panel) = layout.node(*node_id)? else {
            return None;
        };
        let instance = layout.panel_instance(panel.panel_id)?;
        (instance.key == *key).then_some(panel.panel_id)
    })
}
