mod columns;
mod rendering;

use dock_core::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, FloatingGroupId,
    Layout, Node, NodeId, PanelId, PanelKey, Point, Profile, Rect, Size,
};
use dock_gpui::dock_metrics_from_theme;
use ui::Theme;

use crate::panels::PanelRegistry;

#[derive(Clone, Copy)]
struct DockTabContextMenu {
    column_index: usize,
    group: NodeId,
    active_panel: Option<PanelId>,
    header_only: bool,
    left: f32,
    top: f32,
}

/// Runtime view data derived from one Core Tabs node.
struct TabGroupViewData {
    id: NodeId,
    panels: Vec<PanelId>,
    active_panel: Option<PanelId>,
    collapsed: bool,
}

/// UI visibility state is keyed by the Core panel instance, not by a menu index.
struct PanelVisibilityEntry {
    panel_id: PanelId,
    column_index: usize,
    panel_key: String,
    title: String,
    visible: bool,
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

        let layers_key = PanelKey::new("layers").expect("built-in panel key must be valid");
        let color_key = PanelKey::new("color").expect("built-in panel key must be valid");
        let properties_key =
            PanelKey::new("properties").expect("built-in panel key must be valid");

        let constraints_for = |key: &PanelKey, title: &str| {
            panels
                .constraints_for_key(key)
                .ok_or_else(|| format!("{title} panel is missing from the panel registry"))
        };
        let layers_constraints = constraints_for(&layers_key, "Layers")?;
        let color_constraints = constraints_for(&color_key, "Color")?;
        let properties_constraints = constraints_for(&properties_key, "Properties")?;

        let mut layout = Layout::new(Profile::Photoshop);

        // Build the original right column and its two tab groups using Core commands.
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: layers_key.clone(),
                    constraints: layers_constraints,
                    target: None,
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not open the Layers panel: {error}"))?;

        let initial_right_column = layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Dock Core did not create the initial right dock area".to_owned())?
            .root;
        let initial_top_group = group_ids_for_column(&layout, initial_right_column)
            .first()
            .copied()
            .ok_or_else(|| "Dock Core did not create the upper tab group".to_owned())?;

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: color_key.clone(),
                    constraints: color_constraints,
                    target: Some(DropTarget::Tabs {
                        group: initial_top_group,
                        index: 1,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not add the Color tab: {error}"))?;

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: properties_key.clone(),
                    constraints: properties_constraints,
                    target: Some(DropTarget::GroupEdge {
                        group: initial_top_group,
                        zone: DropZone::Bottom,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not create the lower tab group: {error}"))?;

        layout
            .apply(
                DockCommand::RestoreGroupRatios {
                    column: initial_right_column,
                    ratios: vec![split_ratio, 1.0 - split_ratio],
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not apply the right column split ratio: {error}"))?;

        // Insert a second copy to the left of the existing column. Duplicate panel
        // keys are allowed by the registry constraints; each column gets independent
        // panel instances and independent active-tab state.
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: layers_key.clone(),
                    constraints: layers_constraints,
                    target: Some(DropTarget::MainColumn {
                        side: DockSide::Right,
                        index: 0,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not create the left dock column: {error}"))?;

        let right_area = layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Dock Core lost the right dock area".to_owned())?;
        let Node::Split(split) = layout
            .node(right_area.root)
            .ok_or_else(|| "Dock Core did not return the two-column split".to_owned())?
        else {
            return Err("Dock Core did not create a split for the two dock columns".to_owned());
        };
        if split.axis != Axis::X {
            return Err("Dock Core right dock must be a horizontal split of columns".to_owned());
        }
        let column_ids = split.children.clone();
        if column_ids.last() != Some(&initial_right_column) {
            return Err("Dock Core changed the original column's order unexpectedly".to_owned());
        }
        // Initialize each Core group by activating its first actual panel. This
        // does not require a fixed Layers/Color/Properties slot map.
        for column_id in column_ids.iter().copied() {
            let group_ids = match layout.node(column_id) {
                Some(Node::Column(column)) => column.groups.clone(),
                _ => Vec::new(),
            };
            for group_id in group_ids {
                let first_panel = match layout.node(group_id) {
                    Some(Node::Tabs(tabs)) => tabs.children.iter().find_map(|child_id| {
                        match layout.node(*child_id) {
                            Some(Node::Panel(panel)) => Some(panel.panel_id),
                            _ => None,
                        }
                    }),
                    _ => None,
                };
                if let Some(panel) = first_panel {
                    layout
                        .apply(
                            DockCommand::ActivateTab {
                                group: group_id,
                                panel,
                            },
                            &dock_metrics,
                        )
                        .map_err(|error| {
                            format!("Could not activate the first panel in group {group_id:?}: {error}")
                        })?;
                }
            }
        }

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

    fn toggle_tab_context_menu(
        &mut self,
        column_index: usize,
        group: NodeId,
        active_panel: Option<PanelId>,
        pointer_x: f32,
        pointer_y: f32,
    ) {
        if self
            .tab_context_menu
            .is_some_and(|menu| menu.group == group && !menu.header_only)
        {
            self.tab_context_menu = None;
            return;
        }
        self.open_tab_context_menu(column_index, group, active_panel, pointer_x, pointer_y);
    }

    fn open_tab_context_menu(
        &mut self,
        column_index: usize,
        group: NodeId,
        active_panel: Option<PanelId>,
        pointer_x: f32,
        pointer_y: f32,
    ) {
        self.windows_menu_open = false;
        self.tab_context_menu = Some(DockTabContextMenu {
            column_index,
            group,
            active_panel,
            header_only: false,
            left: pointer_x.max(0.0),
            top: pointer_y.max(0.0),
        });
    }

    fn open_header_context_menu(
        &mut self,
        column_index: usize,
        group: NodeId,
        pointer_x: f32,
        pointer_y: f32,
    ) {
        self.windows_menu_open = false;
        self.tab_context_menu = Some(DockTabContextMenu {
            column_index,
            group,
            active_panel: None,
            header_only: true,
            left: pointer_x.max(0.0),
            top: pointer_y.max(0.0),
        });
    }

    fn close_context_tab(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        let Some(panel_id) = menu.active_panel else {
            return;
        };
        if self.panel_is_visible(panel_id) {
            self.toggle_panel_visibility(panel_id);
        }
    }

    fn close_context_tab_group(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        let panel_ids = self
            .tab_group_view_data(menu.group)
            .map(|group| group.panels)
            .unwrap_or_default();
        for panel_id in panel_ids {
            if self.panel_is_visible(panel_id) {
                self.toggle_panel_visibility(panel_id);
            }
        }
    }

    fn context_menu_position(
        menu: DockTabContextMenu,
        window_width: f32,
        window_height: f32,
    ) -> (f32, f32) {
        // Keep the popup under the pointer when possible, but flip its position
        // toward the available space when it approaches a viewport edge.
        let viewport_width = window_width.max(1.0);
        let viewport_height = window_height.max(1.0);
        let popup_width = 184.0_f32.min(viewport_width);
        let popup_height = (if menu.header_only { 42.0_f32 } else { 132.0_f32 }).min(viewport_height);
        let left = menu.left.max(0.0).min((viewport_width - popup_width).max(0.0));
        let top = menu.top.max(0.0).min((viewport_height - popup_height).max(0.0));
        (left, top)
    }

    fn dismiss_menus_if_outside(
        &mut self,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) -> bool {
        let menu_height = self
            .theme
            .metrics
            .column_header_height
            .min(window_height.max(1.0));
        let (context_menu_left, context_menu_top) = self
            .tab_context_menu
            .map(|menu| Self::context_menu_position(menu, window_width, window_height))
            .unwrap_or((0.0, 0.0));
        let inside_context_menu = self.tab_context_menu.is_some_and(|menu| {
            let popup_height = (if menu.header_only { 42.0_f32 } else { 132.0_f32 }).min(window_height.max(1.0));
            pointer_x >= context_menu_left
                && pointer_x <= (context_menu_left + 184.0).min(window_width)
                && pointer_y >= context_menu_top
                && pointer_y <= (context_menu_top + popup_height).min(window_height)
        });
        let inside_windows_menu = self.windows_menu_open
            && pointer_x >= 0.0
            && pointer_x <= 224.0_f32.min(window_width)
            && pointer_y >= menu_height
            && pointer_y <= (menu_height + 136.0).min(window_height);
        // The Windows button itself is a trigger, not an outside click. It can
        // open or close that dropdown while still dismissing the other menu.
        let on_windows_button = pointer_x >= 0.0
            && pointer_x <= 112.0
            && pointer_y >= 0.0
            && pointer_y <= menu_height;
        if inside_context_menu || inside_windows_menu || on_windows_button {
            if on_windows_button && self.tab_context_menu.is_some() {
                self.tab_context_menu = None;
                return true;
            }
            return false;
        }

        let had_open_menu = self.tab_context_menu.is_some() || self.windows_menu_open;
        self.tab_context_menu = None;
        self.windows_menu_open = false;
        had_open_menu
    }

    fn panel_is_visible(&self, panel_id: PanelId) -> bool {
        self.panel_visibility
            .iter()
            .find(|entry| entry.panel_id == panel_id)
            .map_or(true, |entry| entry.visible)
    }

    /// Discover current Core panel instances and ensure each has independent visibility state.
    ///
    /// Existing entries survive when their column temporarily disappears, so closing a
    /// column and restoring it can retain visibility preferences. A newly discovered
    /// panel instance reuses a stale entry only when the stable panel key and column match.
    fn sync_panel_visibility_entries(&mut self) {
        let mut discovered: Vec<(usize, PanelId, String, String)> = Vec::new();

        for (column_index, column_id) in self.column_ids.iter().copied().enumerate() {
            for group_id in self.group_ids_in_column(column_id) {
                let Some(group) = self.tab_group_view_data(group_id) else {
                    continue;
                };
                for panel_id in group.panels {
                    let Some(instance) = self.layout.panel_instance(panel_id) else {
                        continue;
                    };
                    let column_label = self.column_label(column_index);
                    discovered.push((
                        column_index,
                        panel_id,
                        instance.key.as_str().to_owned(),
                        format!("{column_label} {}", self.panels.title(instance)),
                    ));
                }
            }
        }

        let current_panel_ids: Vec<PanelId> =
            discovered.iter().map(|(_, panel_id, _, _)| *panel_id).collect();

        for (column_index, panel_id, panel_key, title) in discovered {
            if let Some(entry) = self
                .panel_visibility
                .iter_mut()
                .find(|entry| entry.panel_id == panel_id)
            {
                entry.column_index = column_index;
                entry.panel_key = panel_key;
                entry.title = title;
                continue;
            }

            let stale_entry_index = self.panel_visibility.iter().position(|entry| {
                entry.column_index == column_index
                    && entry.panel_key == panel_key
                    && !current_panel_ids.contains(&entry.panel_id)
            });

            if let Some(index) = stale_entry_index {
                let entry = &mut self.panel_visibility[index];
                entry.panel_id = panel_id;
                entry.title = title;
            } else {
                self.panel_visibility.push(PanelVisibilityEntry {
                    panel_id,
                    column_index,
                    panel_key,
                    title,
                    visible: true,
                });
            }
        }
    }

    fn toggle_panel_visibility(&mut self, panel_id: PanelId) {
        let Some(entry_index) = self
            .panel_visibility
            .iter()
            .position(|entry| entry.panel_id == panel_id)
        else {
            return;
        };
        let column_index = self.panel_visibility[entry_index].column_index;
        let panel_key = self.panel_visibility[entry_index].panel_key.clone();
        let next_visible = !self.panel_visibility[entry_index].visible;
        self.panel_visibility[entry_index].visible = next_visible;

        if next_visible && !self.column_open[column_index] {
            // Restore the standard dock template, then bind its freshly-created Core
            // panel IDs back to the existing visibility entries by their stable keys.
            if let Err(error) = self.open_column_from_menu(column_index, &panel_key) {
                eprintln!("Could not reopen dock column: {error}");
                if let Some(entry) = self.panel_visibility.get_mut(entry_index) {
                    entry.visible = false;
                }
            }
            return;
        }
        if !self.column_open[column_index] {
            return;
        }

        let column_has_visible_panels = self
            .panel_visibility
            .iter()
            .any(|entry| entry.column_index == column_index && entry.visible);
        if !column_has_visible_panels {
            // Closing the last visible panel closes every group in this column.
            // Core normalization then removes the empty Column and collapses its Split.
            self.close_column(column_index);
            return;
        }

        if next_visible {
            return;
        }

        let group = self
            .group_ids_in_column(self.column_ids[column_index])
            .into_iter()
            .find(|group_id| {
                self.tab_group_view_data(*group_id)
                    .is_some_and(|data| data.panels.contains(&panel_id))
            });
        let Some(group) = group else {
            return;
        };
        if self
            .active_panel_id(group)
            .is_some_and(|active| self.panel_is_visible(active))
        {
            return;
        }
        if let Some(next_panel) = self
            .tab_group_view_data(group)
            .into_iter()
            .flat_map(|data| data.panels)
            .find(|candidate| self.panel_is_visible(*candidate))
        {
            self.activate_tab(group, next_panel);
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

    fn collapse_column_to_icons(&mut self, column_index: usize) {
        if column_index >= self.column_ids.len()
            || matches!(
                self.layout.node(self.column_ids[column_index]),
                Some(Node::Column(column)) if column.mode == ColumnMode::IconsOnly
            )
        {
            return;
        }
        if let Err(error) = self.apply_dock_command(
            DockCommand::SetColumnMode {
                column: self.column_ids[column_index],
                mode: ColumnMode::IconsOnly,
            }) {
            eprintln!("Could not collapse dock column to icons: {error}");
        }
    }

    fn collapse_context_to_icons(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        self.collapse_column_to_icons(menu.column_index);
    }

    fn expand_context_tabs(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        if menu.column_index >= self.column_ids.len() {
            return;
        }
        if let Err(error) = self.apply_dock_command(
            DockCommand::SetColumnMode {
                column: self.column_ids[menu.column_index],
                mode: ColumnMode::Expanded,
            }) {
            eprintln!("Could not expand dock tabs: {error}");
        }
    }

    fn toggle_context_group_collapse(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        let collapsed = !self.group_is_collapsed(menu.group);
        if let Err(error) = self.apply_dock_command(
            DockCommand::CollapseGroup {
                group: menu.group,
                collapsed,
            }) {
            eprintln!("Could not change dock group collapse state: {error}");
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

    fn composition_column_indices(&self, root: NodeId) -> Vec<usize> {
        self.column_indices_in_core_order(root)
    }

    fn composition_width(&self, root: NodeId) -> Option<f32> {
        let columns = self.composition_column_indices(root);
        if columns.is_empty() {
            return None;
        }
        let splitter_gaps = columns.len().saturating_sub(1) as f32;
        let width = columns
            .iter()
            .map(|column_index| self.column_width(*column_index))
            .sum::<f32>()
            + self.theme.metrics.splitter_thickness * splitter_gaps;
        Some(width.max(1.0))
    }

    fn composition_size(&self, root: NodeId, window_height: f32) -> Option<Size> {
        let width = self.composition_width(root)?;
        let menu_height = self
            .theme
            .metrics
            .column_header_height
            .min(window_height.max(1.0));
        let available_height = (window_height - menu_height).max(1.0);
        // Floating docks should be compact enough to move vertically instead
        // of initially filling almost the entire application viewport.
        let floating_height = available_height.min((window_height * 0.75).max(1.0));
        Some(Size::new(width, floating_height.max(1.0)))
    }

    fn sync_floating_composition_bounds(&mut self) {
        let Some(group_id) = self.floating_composition else {
            return;
        };
        let Some((root, current_bounds)) = self
            .layout
            .floating_group(group_id)
            .and_then(|floating| {
                self.layout
                    .dock_area(floating.area)
                    .map(|area| (area.root, floating.bounds))
            })
        else {
            self.floating_composition = None;
            return;
        };
        let Some(width) = self.composition_width(root) else {
            return;
        };
        if (current_bounds.size.width - width).abs() <= f32::EPSILON {
            return;
        }
        let bounds = Rect::new(
            current_bounds.origin,
            Size::new(width, current_bounds.size.height),
        );
        if let Err(error) = self.layout.apply(
            DockCommand::ResizeFloatingBounds { group: group_id, bounds },
            &self.dock_metrics,
        ) {
            eprintln!("Could not synchronize floating dock composition bounds: {error}");
        }
    }

    fn begin_composition_drag(
        &mut self,
        column_index: usize,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) {
        self.composition_drag_pending = false;
        self.composition_dragging = false;
        self.composition_drag_source = None;
        self.composition_drag_group = None;
        self.composition_drag_start_x = pointer_x;
        self.composition_drag_start_y = pointer_y;

        if let Some(group_id) = self.floating_composition {
            let floating_root = self
                .layout
                .floating_group(group_id)
                .and_then(|group| self.layout.dock_area(group.area))
                .map(|area| area.root);
            let clicked_column_is_floating = floating_root.is_some_and(|root| {
                self.composition_column_indices(root).contains(&column_index)
            });

            if clicked_column_is_floating {
                if let Some(origin) = self
                    .layout
                    .floating_group(group_id)
                    .map(|group| group.bounds.origin)
                {
                    self.composition_drag_group = Some(group_id);
                    self.composition_drag_offset_x = pointer_x - origin.x;
                    self.composition_drag_offset_y = pointer_y - origin.y;
                    self.composition_drag_pending = true;
                    return;
                }
                self.floating_composition = None;
            } else if floating_root.is_some() {
                // This UI currently tracks one movable floating composition.
                // Do not let a main-dock header accidentally drag that floating window.
                return;
            } else {
                self.floating_composition = None;
            }
        }

        let Some(root) = self.layout.main_area(DockSide::Right).map(|area| area.root) else {
            return;
        };
        let Some(size) = self.composition_size(root, window_height) else {
            return;
        };
        let origin_x = (window_width - size.width).max(0.0);
        let origin_y = self
            .theme
            .metrics
            .column_header_height
            .min(window_height.max(1.0));
        self.composition_drag_offset_x = pointer_x - origin_x;
        self.composition_drag_offset_y = pointer_y - origin_y;
        self.composition_drag_source = Some(root);
        self.composition_drag_pending = true;
    }

    fn floating_origin_for_pointer(
        &self,
        pointer_x: f32,
        pointer_y: f32,
        size: Size,
        window_width: f32,
        window_height: f32,
    ) -> Point {
        // Keep the floating composition horizontally reachable and its
        // bottom edge inside the application viewport.
        let visible_width = 80.0_f32.min(size.width.max(1.0));
        let min_x = -(size.width - visible_width).max(0.0);
        let max_x = (window_width - visible_width).max(0.0);
        let min_y = 0.0_f32;
        let max_y = (window_height - size.height).max(0.0);
        Point::new(
            (pointer_x - self.composition_drag_offset_x).clamp(min_x, max_x),
            (pointer_y - self.composition_drag_offset_y).clamp(min_y, max_y),
        )
    }

    fn float_main_composition_at_pointer(
        &mut self,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) -> Option<FloatingGroupId> {
        let node = self.composition_drag_source?;
        let size = self.composition_size(node, window_height)?;
        let origin = self.floating_origin_for_pointer(
            pointer_x,
            pointer_y,
            size,
            window_width,
            window_height,
        );
        let bounds = Rect::new(origin, size);
        if let Err(error) = self.apply_dock_command(
            DockCommand::Float { node, bounds }) {
            eprintln!("Could not float dock composition: {error}");
            return None;
        }

        let floating_id = self.layout.floating_groups().find_map(|(group_id, group)| {
            self.layout
                .dock_area(group.area)
                .is_some_and(|area| area.root == node)
                .then_some(group_id)
        })?;
        self.floating_composition = Some(floating_id);
        self.composition_drag_group = Some(floating_id);
        self.composition_drag_source = None;
        Some(floating_id)
    }

    fn move_floating_composition(
        &mut self,
        group_id: FloatingGroupId,
        pointer_x: f32,
        pointer_y: f32,
        window_width: f32,
        window_height: f32,
    ) {
        let Some(size) = self
            .layout
            .floating_group(group_id)
            .map(|group| group.bounds.size)
        else {
            self.floating_composition = None;
            self.composition_drag_group = None;
            self.composition_dragging = false;
            return;
        };
        let origin = self.floating_origin_for_pointer(
            pointer_x,
            pointer_y,
            size,
            window_width,
            window_height,
        );
        if let Err(error) = self.apply_dock_command(
            DockCommand::MoveFloating {
                group: group_id,
                origin,
            }) {
            eprintln!("Could not move floating dock composition: {error}");
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
