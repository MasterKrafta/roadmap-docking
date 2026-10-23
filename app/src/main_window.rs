use dock_core::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, FloatingGroupId,
    Layout, Node, NodeId, PanelId, PanelKey, Point, Profile, Rect, Size,
};
use dock_gpui::dock_metrics_from_theme;
use gpui::{
    Context, CursorStyle, Div, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
    rgb,
};
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

#[derive(Clone, Copy)]
struct GroupResizeState {
    column: NodeId,
    boundary_index: usize,
    available_height: f32,
    splitter_hit_height: f32,
    group_area_top: f32,
}

/// A single-panel left column beside a right column with two upper tabs and one lower tab.
pub(crate) struct MainWindow {
    theme: Theme,
    panels: PanelRegistry,
    layout: Layout,
    dock_metrics: DockMetrics,
    /// Main-dock column IDs in visual order: left, right.
    column_ids: [NodeId; 2],
    /// Core removes empty columns and collapses their parent Split. These flags keep
    /// the view aligned with that topology; the last remaining column stays open.
    column_open: [bool; 2],
    /// The left column has one tab group; the right column has upper and lower groups.
    column_has_bottom_group: [bool; 2],
    top_tab_groups: [NodeId; 2],
    bottom_tab_groups: [NodeId; 2],
    /// Panel IDs per column, ordered: Layers, Color, Properties. Unused slots alias
    /// an existing ID but are never exposed as tabs or menu entries.
    panels_by_column: [[PanelId; 4]; 2],
    /// Visibility choices exposed by the Windows menu. Hidden panels retain
    /// their Core nodes and active-tab state so they can be restored without
    /// destroying their dock location.
    panel_visibility: [bool; 4],
    windows_menu_open: bool,
    tab_context_menu: Option<DockTabContextMenu>,
    is_resizing_column: bool,
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
        let initial_top_group = first_group_in_column(&layout, initial_right_column)
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

        let initial_bottom_group = second_group_in_column(&layout, initial_right_column)
            .ok_or_else(|| "Dock Core did not create the lower tab group".to_owned())?;
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
        if split.axis != Axis::X || split.children.len() != 2 {
            return Err("Dock Core right dock must be a horizontal split of two columns".to_owned());
        }
        let column_ids = [split.children[0], split.children[1]];
        if column_ids[1] != initial_right_column {
            return Err("Dock Core changed the original column's order unexpectedly".to_owned());
        }
        let left_top_group = first_group_in_column(&layout, column_ids[0])
            .ok_or_else(|| "Dock Core did not create the left upper group".to_owned())?;

        let top_tab_groups = [left_top_group, initial_top_group];
        // This ID is a harmless placeholder for the left column, which has no
        // lower group and therefore never renders or targets a bottom splitter.
        let bottom_tab_groups = [left_top_group, initial_bottom_group];
        let panel_id_for_group = |group: NodeId, key: &PanelKey, title: &str| {
            panel_id_in_group(&layout, group, key)
                .ok_or_else(|| format!("Dock Core did not retain {title} in its tab group"))
        };

        let left_layers = panel_id_for_group(left_top_group, &layers_key, "left Layers")?;
        let right_layers = panel_id_for_group(initial_top_group, &layers_key, "right Layers")?;
        let right_color = panel_id_for_group(initial_top_group, &color_key, "right Color")?;
        let right_properties =
            panel_id_for_group(initial_bottom_group, &properties_key, "right Properties")?;
        let column_has_bottom_group = [false, true];
        let panels_by_column = [
            [left_layers, left_layers, left_layers, left_layers],
            [right_layers, right_color, right_properties, right_properties],
        ];

        // The left column has one active tab; the right column has two tab groups.
        for column_index in 0..2 {
            layout
                .apply(
                    DockCommand::ActivateTab {
                        group: top_tab_groups[column_index],
                        panel: panels_by_column[column_index][0],
                    },
                    &dock_metrics,
                )
                .map_err(|error| format!("Could not activate the first tab in column {column_index}: {error}"))?;
            if column_has_bottom_group[column_index] {
                layout
                    .apply(
                        DockCommand::ActivateTab {
                            group: bottom_tab_groups[column_index],
                            panel: panels_by_column[column_index][2],
                        },
                        &dock_metrics,
                    )
                    .map_err(|error| format!("Could not activate Properties in column {column_index}: {error}"))?;
            }
        }

        Ok(Self {
            theme,
            panels,
            layout,
            dock_metrics,
            column_ids,
            column_open: [true; 2],
            column_has_bottom_group,
            top_tab_groups,
            bottom_tab_groups,
            panels_by_column,
            panel_visibility: [true; 4],
            windows_menu_open: false,
            tab_context_menu: None,
            is_resizing_column: false,
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
        })
    }

    /// Apply a Core layout mutation and then reconcile floating composition geometry.
    ///
    /// Keeping this in one path means column mode changes, group removal, and resize
    /// commands all refresh the outer floating bounds from the actual child columns.
    fn apply_dock_command(&mut self, command: DockCommand) -> Result<(), String> {
        self.layout
            .apply(command, &self.dock_metrics)
            .map_err(|error| error.to_string())?;
        self.sync_floating_composition_bounds();
        Ok(())
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
        let Some(panel_slot) = self
            .panels_by_column
            .get(menu.column_index)
            .and_then(|panel_ids| panel_ids.iter().position(|candidate| *candidate == panel_id))
        else {
            return;
        };
        let Some(menu_index) = Self::visibility_index_for_slot(menu.column_index, panel_slot) else {
            return;
        };
        if self.panel_visibility[menu_index] {
            self.toggle_panel_visibility(menu_index);
        }
    }

    fn close_context_tab_group(&mut self) {
        let Some(menu) = self.tab_context_menu.take() else {
            return;
        };
        let menu_indices: &[usize] = if menu.column_index == 0 {
            &[0]
        } else if menu.group == self.top_tab_groups[1] {
            &[1, 2]
        } else if menu.group == self.bottom_tab_groups[1] {
            &[3]
        } else {
            &[]
        };
        for menu_index in menu_indices.iter().copied() {
            if self.panel_visibility[menu_index] {
                self.toggle_panel_visibility(menu_index);
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

    fn visibility_index_for_slot(column_index: usize, panel_slot: usize) -> Option<usize> {
        match (column_index, panel_slot) {
            (0, 0) => Some(0), // Left Layers
            (1, 0) => Some(1), // Right Layers
            (1, 1) => Some(2), // Right Color
            (1, 2) => Some(3), // Right Properties
            _ => None,
        }
    }

    fn panel_slot_visible(&self, column_index: usize, panel_slot: usize) -> bool {
        Self::visibility_index_for_slot(column_index, panel_slot)
            .is_some_and(|visibility_index| self.panel_visibility[visibility_index])
    }

    fn panel_is_visible(&self, column_index: usize, panel_id: PanelId) -> bool {
        let Some(panel_slot) = self
            .panels_by_column
            .get(column_index)
            .and_then(|panel_ids| panel_ids.iter().position(|candidate| *candidate == panel_id))
        else {
            // Panels without a dedicated Windows-menu toggle remain visible by default.
            return true;
        };
        Self::visibility_index_for_slot(column_index, panel_slot)
            .map(|visibility_index| self.panel_visibility[visibility_index])
            .unwrap_or(true)
    }

    fn toggle_panel_visibility(&mut self, menu_index: usize) {
        if menu_index >= self.panel_visibility.len() {
            return;
        }
        let (column_index, panel_slot) = match menu_index {
            0 => (0, 0), // Left Layers
            1 => (1, 0), // Right Layers
            2 => (1, 1), // Right Color
            3 => (1, 2), // Right Properties
            _ => return,
        };

        self.panel_visibility[menu_index] = !self.panel_visibility[menu_index];
        if self.panel_visibility[menu_index] && !self.column_open[column_index] {
            // Recreate the column through Core when its last visible panel was
            // disabled earlier. Rebuild the same three-panel template on the right.
            if let Err(error) = self.open_column_from_menu(column_index, menu_index) {
                eprintln!("Could not reopen dock column: {error}");
                self.panel_visibility[menu_index] = false;
            }
            return;
        }
        if !self.column_open[column_index] {
            return;
        }

        let column_has_visible_panels = match column_index {
            0 => self.panel_visibility[0],
            1 => self.panel_visibility[1] || self.panel_visibility[2] || self.panel_visibility[3],
            _ => false,
        };
        if !column_has_visible_panels {
            // Closing the last visible panel closes every group in this column.
            // Core normalization then removes the empty Column and collapses the
            // surrounding Split, so the other column shifts into its place.
            self.close_column(column_index);
            return;
        }

        let group = if panel_slot < 2 {
            self.top_tab_groups[column_index]
        } else {
            self.bottom_tab_groups[column_index]
        };
        let candidates = if panel_slot < 2 {
            [0usize, 1usize]
        } else {
            [2usize, 3usize]
        };
        let active_visible = self
            .active_panel_id(group)
            .is_some_and(|active| self.panel_is_visible(column_index, active));
        if active_visible {
            return;
        }
        if let Some(next_slot) = candidates
            .into_iter()
            .find(|candidate| self.panel_slot_visible(column_index, *candidate))
        {
            self.activate_tab(group, self.panels_by_column[column_index][next_slot]);
        }
    }

    fn open_column_from_menu(
        &mut self,
        column_index: usize,
        selected_menu_index: usize,
    ) -> Result<(), String> {
        let layers_key = PanelKey::new("layers").expect("built-in panel key is valid");
        let color_key = PanelKey::new("color").expect("built-in panel key is valid");
        let properties_key = PanelKey::new("properties").expect("built-in panel key is valid");
        let layers_constraints = self
            .panels
            .constraints_for_key(&layers_key)
            .ok_or_else(|| "Layers panel is missing from the registry".to_owned())?;
        let color_constraints = self
            .panels
            .constraints_for_key(&color_key)
            .ok_or_else(|| "Color panel is missing from the registry".to_owned())?;
        let properties_constraints = self
            .panels
            .constraints_for_key(&properties_key)
            .ok_or_else(|| "Properties panel is missing from the registry".to_owned())?;

        let target = if self.column_open[1 - column_index] {
            Some(DropTarget::MainColumn {
                side: DockSide::Right,
                index: column_index,
            })
        } else {
            None
        };
        self.layout
            .apply(
                DockCommand::OpenPanel {
                    key: layers_key.clone(),
                    constraints: layers_constraints,
                    target,
                },
                &self.dock_metrics,
            )
            .map_err(|error| format!("Could not recreate Layers: {error}"))?;

        let area = self
            .layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Dock Core did not create a main dock area".to_owned())?;
        let (column_ids, column_open, target_column) = match self.layout.node(area.root) {
            Some(Node::Split(split)) if split.axis == Axis::X && split.children.len() == 2 => {
                let ids = [split.children[0], split.children[1]];
                (ids, [true, true], ids[column_index])
            }
            Some(Node::Column(_)) => {
                let mut ids = self.column_ids;
                ids[column_index] = area.root;
                let mut opened = [false; 2];
                opened[column_index] = true;
                (ids, opened, area.root)
            }
            _ => return Err("Dock Core created an unexpected main dock shape".to_owned()),
        };
        self.column_ids = column_ids;
        self.column_open = column_open;

        let top_group = first_group_in_column(&self.layout, target_column)
            .ok_or_else(|| "Dock Core did not create the upper tab group".to_owned())?;
        let layers_panel = panel_id_in_group(&self.layout, top_group, &layers_key)
            .ok_or_else(|| "Dock Core did not retain Layers in the new column".to_owned())?;

        if column_index == 0 {
            self.top_tab_groups[0] = top_group;
            self.bottom_tab_groups[0] = top_group;
            self.panels_by_column[0] = [layers_panel; 4];
            self.column_has_bottom_group[0] = false;
        } else {
            self.layout
                .apply(
                    DockCommand::OpenPanel {
                        key: color_key.clone(),
                        constraints: color_constraints,
                        target: Some(DropTarget::Tabs {
                            group: top_group,
                            index: 1,
                        }),
                    },
                    &self.dock_metrics,
                )
                .map_err(|error| format!("Could not recreate Color: {error}"))?;
            self.layout
                .apply(
                    DockCommand::OpenPanel {
                        key: properties_key.clone(),
                        constraints: properties_constraints,
                        target: Some(DropTarget::GroupEdge {
                            group: top_group,
                            zone: DropZone::Bottom,
                        }),
                    },
                    &self.dock_metrics,
                )
                .map_err(|error| format!("Could not recreate Properties: {error}"))?;

            let bottom_group = second_group_in_column(&self.layout, target_column)
                .ok_or_else(|| "Dock Core did not create the lower tab group".to_owned())?;
            let color_panel = panel_id_in_group(&self.layout, top_group, &color_key)
                .ok_or_else(|| "Dock Core did not retain Color in the new column".to_owned())?;
            let properties_panel =
                panel_id_in_group(&self.layout, bottom_group, &properties_key)
                    .ok_or_else(|| "Dock Core did not retain Properties in the new column".to_owned())?;

            self.top_tab_groups[1] = top_group;
            self.bottom_tab_groups[1] = bottom_group;
            self.panels_by_column[1] = [
                layers_panel,
                color_panel,
                properties_panel,
                properties_panel,
            ];
            self.column_has_bottom_group[1] = true;

            let selected = match selected_menu_index {
                1 => layers_panel,
                2 => color_panel,
                3 => properties_panel,
                _ => layers_panel,
            };
            let selected_group = if selected_menu_index == 3 {
                bottom_group
            } else {
                top_group
            };
            self.activate_tab(selected_group, selected);
        }
        self.column_open[column_index] = true;
        Ok(())
    }

    fn close_column(&mut self, column_index: usize) {
        if column_index >= self.column_open.len() || !self.column_open[column_index] {
            return;
        }
        for group in [
            self.top_tab_groups[column_index],
            self.bottom_tab_groups[column_index],
        ] {
            if self.layout.node(group).is_none() {
                continue;
            }
            if let Err(error) = self.apply_dock_command(
                DockCommand::CloseGroup { group }) {
                eprintln!("Could not close dock group: {error}");
                return;
            }
        }

        // Core normalization removes the now-empty Column and reduces the
        // two-child Split to the remaining column. The UI stops rendering the
        // removed IDs and lets the surviving column move next to the workspace.
        self.column_open[column_index] = false;
        if column_index == 0 {
            self.panel_visibility[0] = false;
        } else {
            self.panel_visibility[1..=3].fill(false);
        }
        self.is_resizing_column = false;
        self.resizing_outer_column = None;
        self.resizing_groups_column = None;
    }

    fn column_width(&self, column_index: usize) -> f32 {
        self.layout
            .node(self.column_ids[column_index])
            .and_then(|node| match node {
                Node::Column(column) => Some(column.width),
                _ => None,
            })
            .unwrap_or(self.theme.metrics.default_column_width)
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
        match self.layout.node(column_id) {
            Some(Node::Column(column)) => column.groups.clone(),
            _ => Vec::new(),
        }
    }

    fn group_ratios_in_column(&self, column_id: NodeId) -> Vec<f32> {
        match self.layout.node(column_id) {
            Some(Node::Column(column)) => column.group_ratios.clone(),
            _ => Vec::new(),
        }
    }

    fn visible_tab_groups_in_column(
        &self,
        column_index: usize,
        column_id: NodeId,
    ) -> Vec<(usize, TabGroupViewData)> {
        self.group_ids_in_column(column_id)
            .into_iter()
            .enumerate()
            .filter_map(|(group_index, group_id)| {
                let mut group = self.tab_group_view_data(group_id)?;
                group
                    .panels
                    .retain(|panel_id| self.panel_is_visible(column_index, *panel_id));
                if group
                    .active_panel
                    .is_some_and(|active| !group.panels.contains(&active))
                {
                    group.active_panel = None;
                }
                (!group.panels.is_empty()).then_some((group_index, group))
            })
            .collect()
    }

    fn group_heights_for_available_height(
        &self,
        groups: &[(usize, TabGroupViewData)],
        ratios: &[f32],
        available_height: f32,
    ) -> Vec<f32> {
        if groups.is_empty() {
            return Vec::new();
        }

        let available_height = available_height.max(0.0);
        let collapsed_height = self
            .dock_metrics
            .collapsed_group_height
            .max(self.theme.metrics.tab_strip_height)
            .min(available_height);
        let collapsed_count = groups.iter().filter(|(_, group)| group.collapsed).count();
        let fixed_collapsed_height = collapsed_height * collapsed_count as f32;
        let expandable_height = (available_height - fixed_collapsed_height).max(0.0);

        let weights: Vec<f32> = groups
            .iter()
            .map(|(core_index, group)| {
                if group.collapsed {
                    0.0
                } else {
                    ratios
                        .get(*core_index)
                        .copied()
                        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
                        .unwrap_or(1.0)
                }
            })
            .collect();
        let total_weight = weights.iter().sum::<f32>();

        groups
            .iter()
            .zip(weights)
            .map(|((_, group), weight)| {
                if group.collapsed {
                    collapsed_height
                } else if total_weight > 0.0 {
                    expandable_height * weight / total_weight
                } else {
                    0.0
                }
            })
            .collect()
    }

    fn render_group_splitter(
        &self,
        cx: &mut Context<Self>,
        column_id: NodeId,
        boundary_index: usize,
        width: f32,
        hit_height: f32,
        visible_height: f32,
        center_y: f32,
        available_height: f32,
        group_area_top: f32,
    ) -> Div {
        let colors = self.theme.colors;
        div()
            .relative()
            .w(px(width))
            .h(px(hit_height))
            .cursor(CursorStyle::ResizeUpDown)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.is_resizing_column = false;
                    this.resizing_outer_column = None;
                    this.resizing_groups_column = Some(GroupResizeState {
                        column: column_id,
                        boundary_index,
                        available_height,
                        splitter_hit_height: hit_height,
                        group_area_top,
                    });
                    this.resize_group_pointer_offset_y =
                        event.position.y.as_f32() - center_y;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(((hit_height - visible_height) / 2.0).max(0.0)))
                    .w(px(width))
                    .h(px(visible_height))
                    .bg(rgb(colors.splitter)),
            )
    }

    fn composition_column_indices(&self, root: NodeId) -> Vec<usize> {
        match self.layout.node(root) {
            Some(Node::Column(_)) => self
                .column_ids
                .iter()
                .position(|column_id| *column_id == root)
                .into_iter()
                .collect(),
            Some(Node::Split(split)) if split.axis == Axis::X => split
                .children
                .iter()
                .filter_map(|child| {
                    self.column_ids
                        .iter()
                        .position(|column_id| column_id == child)
                })
                .collect(),
            _ => Vec::new(),
        }
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

    fn render_tab(
        &self,
        cx: &mut Context<Self>,
        element_id: String,
        column_index: usize,
        group_id: NodeId,
        panel_id: PanelId,
        width: f32,
        height: f32,
        active: bool,
        title: String,
    ) -> Div {
        let colors = self.theme.colors;

        div()
            .id(element_id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(width))
            .h(px(height))
            .bg(rgb(if active {
                colors.active_tab
            } else {
                colors.dock_background
            }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if active {
                colors.primary_text
            } else {
                colors.muted_text
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.open_tab_context_menu(
                        column_index,
                        group_id,
                        Some(panel_id),
                        event.position.x.as_f32(),
                        event.position.y.as_f32(),
                    );
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        this.toggle_group_collapse(group_id);
                        cx.notify();
                    }
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(group_id, panel_id);
                cx.notify();
            }))
            .child(title)
    }

    fn render_tab_group(
        &self,
        width: f32,
        group_height: f32,
        show_tab_strip: bool,
        show_content: bool,
        content_height: f32,
        tab_strip: Div,
        content: impl IntoElement,
    ) -> Div {
        let colors = self.theme.colors;
        let mut group = div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(group_height));

        if show_tab_strip {
            group = group.child(tab_strip);
        }
        if show_content {
            group = group.child(
                div()
                    .w(px(width))
                    .h(px(content_height))
                    .bg(rgb(colors.group_background))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .child(content),
            );
        }
        group
    }

    fn render_tab_strip(
        &self,
        cx: &mut Context<Self>,
        column_index: usize,
        group_id: NodeId,
        active_panel: Option<PanelId>,
        width: f32,
        height: f32,
        content_padding: f32,
        button_size: f32,
        spacer_width: f32,
        visible_tab_count: usize,
        tabs: Vec<Div>,
        menu_element_id: String,
    ) -> Div {
        let colors = self.theme.colors;

        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(self.render_tab_group_hit_area(
                cx,
                column_index,
                group_id,
                active_panel,
                content_padding,
                height,
            ));

        for tab in tabs {
            strip = strip.child(tab);
        }

        if visible_tab_count == 0 {
            strip = strip.child(
                div()
                    .flex()
                    .items_center()
                    .w(px(spacer_width))
                    .h(px(height))
                    .text_color(rgb(colors.muted_text))
                    .child("No panels"),
            );
        } else {
            strip = strip.child(self.render_tab_group_hit_area(
                cx,
                column_index,
                group_id,
                active_panel,
                spacer_width,
                height,
            ));
        }

        strip
            .child(self.render_tab_group_menu_button(
                cx,
                menu_element_id,
                column_index,
                group_id,
                active_panel,
                button_size,
            ))
            .child(self.render_tab_group_hit_area(
                cx,
                column_index,
                group_id,
                active_panel,
                content_padding,
                height,
            ))
    }

    fn render_tab_group_menu_button(
        &self,
        cx: &mut Context<Self>,
        element_id: String,
        column_index: usize,
        group_id: NodeId,
        active_panel: Option<PanelId>,
        button_size: f32,
    ) -> Div {
        let colors = self.theme.colors;

        div()
            .id(element_id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(button_size))
            .h(px(button_size))
            .text_color(rgb(colors.primary_text))
            .bg(rgb(colors.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.toggle_tab_context_menu(
                        column_index,
                        group_id,
                        active_panel,
                        event.position.x.as_f32(),
                        event.position.y.as_f32(),
                    );
                    cx.notify();
                }),
            )
            .child("☰")
    }

    fn render_tab_group_hit_area(
        &self,
        cx: &mut Context<Self>,
        column_index: usize,
        group_id: NodeId,
        active_panel: Option<PanelId>,
        width: f32,
        height: f32,
    ) -> Div {
        div()
            .w(px(width))
            .h(px(height))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.open_tab_context_menu(
                        column_index,
                        group_id,
                        active_panel,
                        event.position.x.as_f32(),
                        event.position.y.as_f32(),
                    );
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        this.toggle_group_collapse(group_id);
                        cx.notify();
                    }
                }),
            )
    }

    fn render_column(
        &self,
        cx: &mut Context<Self>,
        column_index: usize,
        window_height: f32,
        window_y_offset: f32,
    ) -> Div {
        let column_id = self.column_ids[column_index];
        let core_group_ids = self.group_ids_in_column(column_id);
        // Keep the first current group as the column header's context-menu target.
        let top_tab_group = core_group_ids
            .first()
            .copied()
            .unwrap_or(self.top_tab_groups[column_index]);
        let panel_ids = self.panels_by_column[column_index];
        let column_width = self.column_width(column_index);
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let tabs_collapsed = matches!(
            self.layout.node(column_id),
            Some(Node::Column(column)) if column.mode == ColumnMode::IconsOnly
        );

        let header_height = metrics.column_header_height.min(window_height);
        let available_below_header = (window_height - header_height).max(0.0);
        let visible_groups = self.visible_tab_groups_in_column(column_index, column_id);
        // ResizeGroups operates on adjacent Core groups. Avoid exposing splitters
        // while any Core group is hidden from the view by a panel-visibility toggle.
        let all_groups_visible = visible_groups.len() == core_group_ids.len();
        let divider_count = if all_groups_visible {
            visible_groups.len().saturating_sub(1)
        } else {
            0
        };
        let requested_splitter_hit_height =
            metrics.splitter_hit_zone.max(metrics.splitter_thickness);
        let group_split_hit_height = if divider_count > 0 {
            requested_splitter_hit_height.min(
                (available_below_header / divider_count as f32).max(0.0),
            )
        } else {
            0.0
        };
        let visible_split_height = metrics.splitter_thickness.min(group_split_hit_height);
        let groups_height =
            (available_below_header - group_split_hit_height * divider_count as f32).max(0.0);
        let group_ratios = self.group_ratios_in_column(column_id);
        let group_heights =
            self.group_heights_for_available_height(&visible_groups, &group_ratios, groups_height);

        let button_size = metrics.button_size.min(column_width);
        let button_gap = if tabs_collapsed { 0.0 } else { metrics.button_gap };
        let content_padding = if tabs_collapsed {
            0.0
        } else {
            metrics.content_padding.min(column_width / 2.0)
        };
        let collapse_button_width = if tabs_collapsed {
            (column_width * 0.58).max(1.0)
        } else {
            button_size
        };
        let close_button_width = if tabs_collapsed {
            (column_width - collapse_button_width).max(1.0)
        } else {
            button_size
        };
        let header_spacer_width = if tabs_collapsed {
            0.0
        } else {
            (column_width - content_padding - button_size * 2.0 - button_gap).max(0.0)
        };
        let tab_rail_width = if tabs_collapsed { column_width } else { 0.0 };
        let column_content_width = (column_width - tab_rail_width).max(0.0);
        let rail_item_width = if tabs_collapsed { column_width } else { button_size };
        let available_tab_width =
            (column_content_width - content_padding * 2.0 - button_size).max(0.0);

        let column_label = if column_index == 0 { "Left" } else { "Right" };
        let mut rendered_groups: Vec<(usize, Div, f32)> = Vec::new();
        if !tabs_collapsed {
            for (visible_index, ((group_index, group), group_height)) in
                visible_groups.iter().zip(group_heights.iter().copied()).enumerate()
            {
                let visible_tab_count = group.panels.len();
                let tab_width = metrics
                    .column_tab_width
                    .min((available_tab_width / visible_tab_count.max(1) as f32).max(0.0));
                let spacer_width =
                    (available_tab_width - tab_width * visible_tab_count as f32).max(0.0);
                let tab_strip_height = metrics.tab_strip_height.min(group_height);
                let content_height = if group.collapsed {
                    0.0
                } else {
                    (group_height - tab_strip_height).max(0.0)
                };

                let mut tabs = Vec::with_capacity(visible_tab_count);
                for (tab_index, panel_id) in group.panels.iter().copied().enumerate() {
                    let Some(panel) = self.layout.panel_instance(panel_id) else {
                        continue;
                    };
                    let slot = panel_ids.iter().position(|candidate| *candidate == panel_id);
                    let element_id = match slot {
                        Some(0) => format!("{column_label.to_lowercase()}-dock-tab-layers"),
                        Some(1) => format!("{column_label.to_lowercase()}-dock-tab-color"),
                        Some(2) => format!("{column_label.to_lowercase()}-dock-tab-properties"),
                        _ => format!(
                            "dock-column-{column_index}-group-{group_index}-tab-{tab_index}"
                        ),
                    };
                    let title = format!("{column_label} {}", self.panels.title(panel));
                    tabs.push(self.render_tab(
                        cx,
                        element_id,
                        column_index,
                        group.id,
                        panel_id,
                        tab_width,
                        tab_strip_height,
                        group.active_panel == Some(panel_id),
                        title,
                    ));
                }

                let menu_element_id = match (column_index, *group_index) {
                    (0, 0) => "left-dock-top-group-menu".to_owned(),
                    (1, 0) => "right-dock-top-group-menu".to_owned(),
                    (1, 1) => "right-dock-bottom-group-menu".to_owned(),
                    _ => format!("dock-column-{column_index}-group-{group_index}-menu"),
                };
                let tab_strip = self.render_tab_strip(
                    cx,
                    column_index,
                    group.id,
                    group.active_panel,
                    column_content_width,
                    tab_strip_height,
                    content_padding,
                    button_size,
                    spacer_width,
                    visible_tab_count,
                    tabs,
                    menu_element_id,
                );
                let content = group
                    .active_panel
                    .and_then(|panel_id| self.layout.panel_instance(panel_id))
                    .map(|panel| self.panels.render(panel, &self.theme))
                    .unwrap_or_else(|| div().into_any_element());
                let group_view = self.render_tab_group(
                    column_content_width,
                    group_height,
                    true,
                    !group.collapsed,
                    content_height,
                    tab_strip,
                    content,
                );
                rendered_groups.push((*group_index, group_view, group_height));
                let _ = visible_index;
            }
        }

        // The same visible group model feeds the icon rail. No separate top/bottom
        // list is needed, so newly added Core groups automatically appear here.
        let collapsed_tabs: Vec<(usize, NodeId, PanelId, bool)> = visible_groups
            .iter()
            .flat_map(|(group_index, group)| {
                group.panels.iter().copied().map(move |panel_id| {
                    (
                        *group_index,
                        group.id,
                        panel_id,
                        group.active_panel == Some(panel_id),
                    )
                })
            })
            .collect();

        let mut column_view = div()
            .relative()
            .flex()
            .flex_col()
            .w(px(column_width))
            .h(px(window_height))
            .bg(rgb(colors.dock_background))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w(px(column_width))
                    .h(px(header_height))
                    .bg(rgb(colors.toolbar_background))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .child(
                        div()
                            .id(if column_index == 0 {
                                "left-dock-header-spacer"
                            } else {
                                "right-dock-header-spacer"
                            })
                            .w(px(header_spacer_width))
                            .h(px(header_height))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.open_header_context_menu(
                                        column_index,
                                        top_tab_group,
                                        event.position.x.as_f32(),
                                        event.position.y.as_f32(),
                                    );
                                    cx.notify();
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    if event.click_count == 2 {
                                        this.composition_drag_pending = false;
                                        this.composition_dragging = false;
                                        this.composition_drag_source = None;
                                        this.composition_drag_group = None;
                                        this.toggle_column_mode(column_index);
                                    } else {
                                        let viewport = window.viewport_size();
                                        this.begin_composition_drag(
                                            event.position.x.as_f32(),
                                            event.position.y.as_f32(),
                                            viewport.width.as_f32(),
                                            viewport.height.as_f32(),
                                        );
                                    }
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id(if column_index == 0 {
                                "left-dock-collapse-tabs"
                            } else {
                                "right-dock-collapse-tabs"
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(collapse_button_width))
                            .h(px(button_size))
                            .bg(rgb(colors.hover))
                            .text_color(rgb(colors.primary_text))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.open_header_context_menu(
                                        column_index,
                                        top_tab_group,
                                        event.position.x.as_f32(),
                                        event.position.y.as_f32(),
                                    );
                                    cx.notify();
                                }),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_column_mode(column_index);
                                cx.notify();
                            }))
                            .child(if tabs_collapsed { "<<" } else { ">>" }),
                    )
                    .child(div().w(px(button_gap)).h(px(1.0)))
                    .child(
                        div()
                            .id(if column_index == 0 {
                                "left-dock-close-column"
                            } else {
                                "right-dock-close-column"
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(close_button_width))
                            .h(px(button_size))
                            .bg(rgb(colors.hover))
                            .text_color(rgb(colors.primary_text))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_column(column_index);
                                cx.notify();
                            }))
                            .child("×"),
                    )
                    .child(div().w(px(content_padding)).h(px(1.0))),
            );
        if !tabs_collapsed {
            let group_area_top = window_y_offset + header_height;
            let mut group_cursor_y = group_area_top;
            let mut previous_group_index: Option<usize> = None;
            for (group_index, group_view, group_height) in rendered_groups {
                if all_groups_visible {
                    if let Some(previous_index) = previous_group_index {
                        let center_y = group_cursor_y + group_split_hit_height / 2.0;
                        column_view = column_view.child(self.render_group_splitter(
                            cx,
                            column_id,
                            previous_index,
                            column_content_width,
                            group_split_hit_height,
                            visible_split_height,
                            center_y,
                            groups_height,
                            group_area_top,
                        ));
                        group_cursor_y += group_split_hit_height;
                    }
                }
                column_view = column_view.child(group_view);
                group_cursor_y += group_height;
                previous_group_index = Some(group_index);
            }
        }
        if tabs_collapsed {
            let mut collapsed_tab_rail = div()
                .absolute()
                .right(px(0.0))
                .top(px(header_height))
                .flex()
                .flex_col()
                .items_center()
                .w(px(tab_rail_width))
                .h(px(available_below_header))
                .bg(rgb(colors.dock_background))
                .border_1()
                .border_color(rgb(colors.border));
            let mut last_group_index: Option<usize> = None;
            for (tab_index, (group_index, group_id, panel_id, active_panel)) in
                collapsed_tabs.iter().copied().enumerate()
            {
                let Some(panel) = self.layout.panel_instance(panel_id) else {
                    continue;
                };
                // Each tab group gets one mini-header above all of its icons.
                // This separate strip can later host group dragging and double-click.
                if last_group_index != Some(group_index) {
                    if last_group_index.is_some() {
                        let group_separator = div()
                            .id(format!("collapsed-dock-group-separator-{column_index}-{group_index}"))
                            .w(px(rail_item_width))
                            .h(px(2.0))
                            .bg(rgb(colors.border));
                        collapsed_tab_rail = collapsed_tab_rail.child(group_separator);
                    }
                    let mini_header = div()
                        .id(format!("collapsed-dock-group-header-{column_index}-{group_index}"))
                        .w(px(rail_item_width))
                        .h(px(12.0))
                        .bg(rgb(colors.dock_background))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.open_header_context_menu(
                                    column_index,
                                    group_id,
                                    event.position.x.as_f32(),
                                    event.position.y.as_f32(),
                                );
                                cx.notify();
                            }),
                        );
                    collapsed_tab_rail = collapsed_tab_rail.child(mini_header);
                    last_group_index = Some(group_index);
                }

                let tab_icon = self.panels.icon(panel).to_owned();
                let tab = div()
                    .id(format!("collapsed-dock-tab-{column_index}-{tab_index}"))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_size))
                    .h(px(button_size))
                    .bg(rgb(colors.dock_background))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .text_color(rgb(if active_panel {
                        colors.primary_text
                    } else {
                        colors.muted_text
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_tab(group_id, panel_id);
                        cx.notify();
                    }))
                    .child(tab_icon);
                collapsed_tab_rail = collapsed_tab_rail.child(tab);
            }
            column_view = column_view.child(collapsed_tab_rail);
        }
        column_view
    }
}

fn first_group_in_column(layout: &Layout, column_id: NodeId) -> Option<NodeId> {
    match layout.node(column_id)? {
        Node::Column(column) => column.groups.first().copied(),
        _ => None,
    }
}

fn second_group_in_column(layout: &Layout, column_id: NodeId) -> Option<NodeId> {
    match layout.node(column_id)? {
        Node::Column(column) => column.groups.get(1).copied(),
        _ => None,
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

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let window_width = viewport.width.as_f32().max(1.0);
        let window_height = viewport.height.as_f32().max(1.0);
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let menu_height = metrics.column_header_height.min(window_height);
        let dock_height = (window_height - menu_height).max(1.0);

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
        let composition_is_floating = self.floating_composition.is_some();
        let left_is_open = self.column_open[0] && !composition_is_floating;
        let right_is_open = self.column_open[1] && !composition_is_floating;
        let any_column_open = left_is_open || right_is_open;
        let both_columns_open = left_is_open && right_is_open;
        let left_width = if left_is_open {
            self.column_width(0).min(window_width)
        } else {
            0.0
        };
        let right_width = if right_is_open {
            self.column_width(1).min(window_width)
        } else {
            0.0
        };
        let splitter_width = metrics.splitter_thickness;
        let outer_splitter_width = if any_column_open { splitter_width } else { 0.0 };
        let center_splitter_width = if both_columns_open { splitter_width } else { 0.0 };
        let dock_width =
            left_width + outer_splitter_width + center_splitter_width + right_width;
        let workspace_width = (window_width - dock_width).max(0.0);
        let split_hit_width = metrics.splitter_hit_zone.max(splitter_width);

        let left_column = if left_is_open {
            self.render_column(cx, 0, dock_height, menu_height)
        } else {
            div().w(px(0.0)).h(px(dock_height))
        };
        let right_column = if right_is_open {
            self.render_column(cx, 1, dock_height, menu_height)
        } else {
            div().w(px(0.0)).h(px(dock_height))
        };
        let outer_splitter_center_x = workspace_width + outer_splitter_width / 2.0;
        let outer_column_index = if left_is_open { 0 } else { 1 };
        let outer_column_id = self.column_ids[outer_column_index];
        let outer_column_splitter = if any_column_open {
            div()
                .relative()
                .w(px(outer_splitter_width))
                .h(px(dock_height))
                .bg(rgb(colors.splitter))
                .child(
                    div()
                        .absolute()
                        .left(px((outer_splitter_width - split_hit_width) / 2.0))
                        .top(px(0.0))
                        .w(px(split_hit_width))
                        .h(px(dock_height))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                                this.is_resizing_column = false;
                                this.resizing_outer_column = Some(outer_column_id);
                                this.resize_column_pointer_offset_x =
                                    event.position.x.as_f32() - outer_splitter_center_x;
                                this.resizing_groups_column = None;
                                cx.notify();
                            }),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px((split_hit_width - outer_splitter_width) / 2.0))
                                .top(px(0.0))
                                .w(px(outer_splitter_width))
                                .h(px(dock_height))
                                .bg(rgb(colors.splitter)),
                        ),
                )
        } else {
            div().w(px(0.0)).h(px(dock_height))
        };
        let column_splitter_center_x =
            workspace_width + outer_splitter_width + left_width + splitter_width / 2.0;
        let column_splitter = if both_columns_open {
            div()
                .relative()
                .w(px(splitter_width))
                .h(px(dock_height))
                .bg(rgb(colors.splitter))
                .child(
                    div()
                        .absolute()
                        .left(px((splitter_width - split_hit_width) / 2.0))
                        .top(px(0.0))
                        .w(px(split_hit_width))
                        .h(px(dock_height))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                                this.is_resizing_column = true;
                                this.resizing_outer_column = None;
                                this.resize_column_pointer_offset_x =
                                    event.position.x.as_f32() - column_splitter_center_x;
                                this.resizing_groups_column = None;
                                cx.notify();
                            }),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px((split_hit_width - splitter_width) / 2.0))
                                .top(px(0.0))
                                .w(px(splitter_width))
                                .h(px(dock_height))
                                .bg(rgb(colors.splitter)),
                        ),
                )

        } else {
            div().w(px(0.0)).h(px(dock_height))
        };
        let windows_button = div()
            .id("windows-menu-button")
            .flex()
            .items_center()
            .justify_center()
            .h(px(menu_height))
            .p(px(8.0))
            .bg(rgb(if self.windows_menu_open { colors.hover } else { colors.toolbar_background }))
            .text_color(rgb(colors.primary_text))
            .on_click(cx.listener(|this, _, _, cx| {
                this.tab_context_menu = None;
                this.windows_menu_open = !this.windows_menu_open;
                cx.notify();
            }))
            .child("Windows ▾");
        let menu_bar = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(window_width))
            .h(px(menu_height))
            .bg(rgb(colors.toolbar_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(windows_button)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(menu_height))
                    .p(px(8.0))
                    .text_color(rgb(colors.muted_text))
                    .child("Roadmap Docking"),
            );

        let dock_area = div()
            .flex()
            .flex_row()
            .w(px(window_width))
            .h(px(dock_height))
            .bg(rgb(colors.canvas_background))
            .child(div().w(px(workspace_width)).h(px(dock_height)))
            .child(outer_column_splitter)
            .child(left_column)
            .child(column_splitter)
            .child(right_column);

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .w(px(window_width))
            .h(px(window_height))
            .bg(rgb(colors.canvas_background))
            .child(menu_bar)
            .child(dock_area);

        if let Some((composition_root, bounds)) = self.floating_composition.and_then(|group_id| {
            let floating = self.layout.floating_group(group_id)?;
            let area = self.layout.dock_area(floating.area)?;
            Some((area.root, floating.bounds))
        }) {
            let composition_columns = self.composition_column_indices(composition_root);
            if !composition_columns.is_empty() {
                let mut floating_view = div()
                    .id("floating-dock-composition")
                    .absolute()
                    .left(px(bounds.origin.x))
                    .top(px(bounds.origin.y))
                    .flex()
                    .flex_row()
                    .w(px(bounds.size.width))
                    .h(px(bounds.size.height))
                    .bg(rgb(colors.dock_background))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .overflow_hidden();
                for (position, column_index) in composition_columns.iter().copied().enumerate() {
                    if position > 0 {
                        floating_view = floating_view.child(
                            div()
                                .w(px(metrics.splitter_thickness))
                                .h(px(bounds.size.height))
                                .bg(rgb(colors.splitter)),
                        );
                    }
                    floating_view = floating_view.child(self.render_column(
                        cx,
                        column_index,
                        bounds.size.height,
                        bounds.origin.y,
                    ));
                }
                root = root.child(floating_view);
            }
        }

        if let Some(menu_state) = self.tab_context_menu {
            let (context_menu_left, context_menu_top) =
                Self::context_menu_position(menu_state, window_width, window_height);
            let context_menu_width = 184.0_f32.min(window_width);
            let context_item_width = (context_menu_width - 8.0).max(1.0).min(176.0);
            let close_tab = div()
                .id("dock-context-close-tab")
                .flex()
                .items_center()
                .w(px(context_item_width))
                .h(px(30.0))
                .p(px(8.0))
                .bg(rgb(colors.toolbar_background))
                .text_color(rgb(colors.primary_text))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_context_tab();
                    cx.notify();
                }))
                .child("Close Tab");
            let group_is_collapsed = self.group_is_collapsed(menu_state.group);
            let group_action_title = if group_is_collapsed {
                "Expand Tab Group"
            } else {
                "Minimize"
            };
            let minimize = div()
                .id("dock-context-minimize")
                .flex()
                .items_center()
                .w(px(context_item_width))
                .h(px(30.0))
                .p(px(8.0))
                .bg(rgb(colors.toolbar_background))
                .text_color(rgb(colors.primary_text))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_context_group_collapse();
                    cx.notify();
                }))
                .child(group_action_title);
            let collapse_to_icons = div()
                .id("dock-context-collapse-to-icons")
                .flex()
                .items_center()
                .w(px(context_item_width))
                .h(px(30.0))
                .p(px(8.0))
                .bg(rgb(colors.toolbar_background))
                .text_color(rgb(colors.primary_text))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.collapse_context_to_icons();
                    cx.notify();
                }))
                .child("Collapse to Icons");
            let expand_tabs = div()
                .id("dock-context-expand-tabs")
                .flex()
                .items_center()
                .w(px(context_item_width))
                .h(px(30.0))
                .p(px(8.0))
                .bg(rgb(colors.toolbar_background))
                .text_color(rgb(colors.primary_text))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.expand_context_tabs();
                    cx.notify();
                }))
                .child("Expand Tabs");
            let close_tab_group = div()
                .id("dock-context-close-tab-group")
                .flex()
                .items_center()
                .w(px(context_item_width))
                .h(px(30.0))
                .p(px(8.0))
                .bg(rgb(colors.toolbar_background))
                .text_color(rgb(colors.primary_text))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_context_tab_group();
                    cx.notify();
                }))
                .child("Close Tab Group");
            let mut dropdown = div()
                .id("dock-tab-context-menu")
                .absolute()
                .left(px(context_menu_left))
                .top(px(context_menu_top))
                .flex()
                .flex_col()
                .w(px(context_menu_width))
                .bg(rgb(colors.toolbar_background))
                .border_1()
                .border_color(rgb(colors.border))
                .p(px(4.0));
            if menu_state.header_only {
                let column_is_collapsed = matches!(
                    self.layout.node(self.column_ids[menu_state.column_index]),
                    Some(Node::Column(column)) if column.mode == ColumnMode::IconsOnly
                );
                dropdown = if column_is_collapsed {
                    dropdown.child(expand_tabs)
                } else {
                    dropdown.child(collapse_to_icons)
                };
            } else {
                dropdown = dropdown
                    .child(close_tab)
                    .child(close_tab_group)
                    .child(minimize)
                    .child(collapse_to_icons);
            }
            root = root.child(dropdown);
        }

        if self.windows_menu_open {
            let mut dropdown = div()
                .id("windows-panel-dropdown")
                .absolute()
                .left(px(0.0))
                .top(px(menu_height))
                .w(px(220.0))
                .bg(rgb(colors.toolbar_background))
                .border_1()
                .border_color(rgb(colors.border))
                .p(px(4.0));
            let panel_titles = [
                "Left Layers",
                "Right Layers",
                "Right Color",
                "Right Properties",
            ];
            for (index, title) in panel_titles.into_iter().enumerate() {
                let visible = self.panel_visibility[index];
                let panel_index = index;
                let item_id = match index {
                    0 => "windows-panel-left-layers",
                    1 => "windows-panel-right-layers",
                    2 => "windows-panel-right-color",
                    _ => "windows-panel-right-properties",
                };
                let item = div()
                    .id(item_id)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w(px(210.0))
                    .h(px(30.0))
                    .p(px(6.0))
                    .bg(rgb(colors.toolbar_background))
                    .text_color(rgb(if visible { colors.primary_text } else { colors.muted_text }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_panel_visibility(panel_index);
                        cx.notify();
                    }))
                    .child(if visible { "☑" } else { "☐" })
                    .child(title);
                dropdown = dropdown.child(item);
            }
            root = root.child(dropdown);
        }

        root.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                let size = window.viewport_size();
                let changed = this.dismiss_menus_if_outside(
                    event.position.x.as_f32(),
                    event.position.y.as_f32(),
                    size.width.as_f32(),
                    size.height.as_f32(),
                );
                if changed {
                    cx.notify();
                }
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                let size = window.viewport_size();
                let changed = this.dismiss_menus_if_outside(
                    event.position.x.as_f32(),
                    event.position.y.as_f32(),
                    size.width.as_f32(),
                    size.height.as_f32(),
                );
                if changed {
                    cx.notify();
                }
            }),
        )
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let pointer_x = event.position.x.as_f32();
                let pointer_y = event.position.y.as_f32();
                let viewport = window.viewport_size();
                let window_width = viewport.width.as_f32().max(1.0);
                let window_height = viewport.height.as_f32().max(1.0);

                if this.composition_drag_pending {
                    let delta_x = pointer_x - this.composition_drag_start_x;
                    let delta_y = pointer_y - this.composition_drag_start_y;
                    if delta_x * delta_x + delta_y * delta_y >= 16.0 {
                        let floating_group = if let Some(group_id) = this.composition_drag_group {
                            Some(group_id)
                        } else {
                            this.float_main_composition_at_pointer(
                                pointer_x,
                                pointer_y,
                                window_width,
                                window_height,
                            )
                        };
                        if let Some(group_id) = floating_group {
                            this.composition_drag_group = Some(group_id);
                            this.composition_dragging = true;
                            this.composition_drag_pending = false;
                            this.move_floating_composition(
                                group_id,
                                pointer_x,
                                pointer_y,
                                window_width,
                                window_height,
                            );
                        } else {
                            this.composition_drag_pending = false;
                            this.composition_drag_source = None;
                            this.composition_drag_group = None;
                        }
                        cx.notify();
                    }
                    return;
                }

                if this.composition_dragging {
                    if let Some(group_id) = this.composition_drag_group {
                        this.move_floating_composition(
                            group_id,
                            pointer_x,
                            pointer_y,
                            window_width,
                            window_height,
                        );
                        cx.notify();
                    }
                    return;
                }

                if let Some(column_id) = this.resizing_outer_column {
                    let pointer_x = event.position.x.as_f32();
                    let window_width = window.viewport_size().width.as_f32().max(1.0);
                    let splitter_width = this.theme.metrics.splitter_thickness;
                    let left_is_open = this.column_open[0];
                    let right_is_open = this.column_open[1];
                    let left_width = if left_is_open {
                        this.column_width(0).min(window_width)
                    } else {
                        0.0
                    };
                    let right_width = if right_is_open {
                        this.column_width(1).min(window_width)
                    } else {
                        0.0
                    };
                    let outer_splitter_width = splitter_width;
                    let center_splitter_width =
                        if left_is_open && right_is_open { splitter_width } else { 0.0 };
                    let workspace_width = (window_width
                        - left_width
                        - right_width
                        - center_splitter_width
                        - outer_splitter_width)
                        .max(0.0);
                    let current_splitter_center_x =
                        workspace_width + outer_splitter_width / 2.0;
                    let target_splitter_center_x =
                        pointer_x - this.resize_column_pointer_offset_x;
                    let current_outer_width = this.column_width(if left_is_open { 0 } else { 1 });
                    let width_delta = current_splitter_center_x - target_splitter_center_x;
                    let requested_width = (current_outer_width + width_delta).max(1.0);
                    let max_width = (window_width
                        - (if left_is_open { right_width } else { 0.0 })
                        - center_splitter_width
                        - outer_splitter_width
                        - this.theme.metrics.min_document_width)
                        .max(0.0);
                    // Keep a usable workspace even when Core has no measured canvas
                    // bounds yet. If the current column already exceeds this limit
                    // (for example after a window shrink), allow dragging inward.
                    let requested_width = if max_width >= current_outer_width {
                        requested_width.min(max_width.max(1.0))
                    } else if width_delta > 0.0 {
                        current_outer_width
                    } else {
                        requested_width
                    };
                    if width_delta != 0.0 {
                        if let Err(error) = this.apply_dock_command(
                            DockCommand::ResizeColumn {
                                column: column_id,
                                width: requested_width,
                            }) {
                            eprintln!("Could not resize outer dock column: {error}");
                            this.resizing_outer_column = None;
                        }
                    }
                    cx.notify();
                    return;
                }

                if this.is_resizing_column {
                    let pointer_x = event.position.x.as_f32();
                    let window_width = window.viewport_size().width.as_f32().max(1.0);
                    let splitter_width = this.theme.metrics.splitter_thickness;
                    let left_width = this.column_width(0).min(window_width);
                    let right_width = this.column_width(1).min(window_width);
                    let outer_splitter_width = splitter_width;
                    let workspace_width = (window_width
                        - left_width
                        - splitter_width
                        - outer_splitter_width
                        - right_width)
                        .max(0.0);
                    let current_splitter_center_x =
                        workspace_width + outer_splitter_width + left_width + splitter_width / 2.0;
                    let target_splitter_center_x =
                        pointer_x - this.resize_column_pointer_offset_x;
                    let delta_x = target_splitter_center_x - current_splitter_center_x;
                    if this.column_open.iter().all(|is_open| *is_open) && delta_x != 0.0 {
                        if let Err(error) = this.apply_dock_command(
                            DockCommand::ResizeColumnBoundary {
                                side: DockSide::Right,
                                boundary_index: 0,
                                delta: delta_x,
                            }) {
                            eprintln!("Could not resize adjacent dock columns: {error}");
                            this.is_resizing_column = false;
                        }
                    }
                    cx.notify();
                    return;
                }

                if let Some(resize) = this.resizing_groups_column {
                    let pointer_y = event.position.y.as_f32();
                    let column_id = resize.column;
                    let Some(column_index) = this
                        .column_ids
                        .iter()
                        .position(|candidate| *candidate == column_id)
                    else {
                        this.resizing_groups_column = None;
                        cx.notify();
                        return;
                    };

                    let core_group_ids = this.group_ids_in_column(column_id);
                    let visible_groups = this.visible_tab_groups_in_column(column_index, column_id);
                    if visible_groups.len() != core_group_ids.len()
                        || resize.boundary_index >= core_group_ids.len().saturating_sub(1)
                    {
                        this.resizing_groups_column = None;
                        cx.notify();
                        return;
                    }

                    let group_ratios = this.group_ratios_in_column(column_id);
                    let group_heights = this.group_heights_for_available_height(
                        &visible_groups,
                        &group_ratios,
                        resize.available_height,
                    );
                    let groups_before_boundary = group_heights
                        .iter()
                        .take(resize.boundary_index + 1)
                        .sum::<f32>();
                    let current_splitter_center_y = resize.group_area_top
                        + groups_before_boundary
                        + resize.splitter_hit_height * resize.boundary_index as f32
                        + resize.splitter_hit_height / 2.0;
                    let target_splitter_center_y =
                        pointer_y - this.resize_group_pointer_offset_y;
                    let delta_y = target_splitter_center_y - current_splitter_center_y;
                    if delta_y != 0.0 {
                        if let Err(error) = this.apply_dock_command(
                            DockCommand::ResizeGroups {
                                column: column_id,
                                boundary_index: resize.boundary_index,
                                delta: delta_y,
                                available_height: resize.available_height,
                            }) {
                            eprintln!("Could not resize dock groups: {error}");
                            this.resizing_groups_column = None;
                        }
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    let had_composition_drag =
                        this.composition_drag_pending || this.composition_dragging;
                    this.composition_drag_pending = false;
                    this.composition_dragging = false;
                    this.composition_drag_source = None;
                    this.composition_drag_group = None;
                    if had_composition_drag
                        || this.is_resizing_column
                        || this.resizing_outer_column.is_some()
                        || this.resizing_groups_column.is_some()
                    {
                        this.is_resizing_column = false;
                        this.resizing_outer_column = None;
                        this.resizing_groups_column = None;
                        cx.notify();
                    }
                }),
            )
    }
}
