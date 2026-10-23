use dock_core::{
    Axis, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, Layout, Node, NodeId, PanelId,
    PanelKey, Profile,
};
use dock_gpui::dock_metrics_from_theme;
use gpui::{
    Context, CursorStyle, Div, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
    rgb,
};
use ui::Theme;

use crate::panels::PanelRegistry;

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
    /// Whether each column renders tabs as a compact vertical icon rail.
    column_tabs_collapsed: [bool; 2],
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
    is_resizing_column: bool,
    /// The outermost dock column whose canvas-facing edge is being resized.
    resizing_outer_column: Option<NodeId>,
    /// Cursor-to-divider offset captured on mouse-down; keeps the pointer anchored
    /// to the actual boundary even after Core clamps the requested resize.
    resize_column_pointer_offset_x: f32,
    resizing_groups_column: Option<NodeId>,
    resize_group_pointer_offset_y: f32,
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
            column_tabs_collapsed: [false; 2],
            column_has_bottom_group,
            top_tab_groups,
            bottom_tab_groups,
            panels_by_column,
            panel_visibility: [true; 4],
            windows_menu_open: false,
            is_resizing_column: false,
            resizing_outer_column: None,
            resize_column_pointer_offset_x: 0.0,
            resizing_groups_column: None,
            resize_group_pointer_offset_y: 0.0,
        })
    }

    fn activate_tab(&mut self, group: NodeId, panel: PanelId) {
        if let Err(error) = self.layout.apply(
            DockCommand::ActivateTab { group, panel },
            &self.dock_metrics,
        ) {
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
        self.panels_by_column
            .get(column_index)
            .and_then(|panel_ids| panel_ids.iter().position(|candidate| *candidate == panel_id))
            .is_some_and(|panel_slot| self.panel_slot_visible(column_index, panel_slot))
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
            if let Err(error) = self.layout.apply(
                DockCommand::CloseGroup { group },
                &self.dock_metrics,
            ) {
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

    fn group_split_ratio(&self, column_index: usize) -> f32 {
        self.layout
            .node(self.column_ids[column_index])
            .and_then(|node| match node {
                Node::Column(column) => column.group_ratios.first().copied(),
                _ => None,
            })
            .unwrap_or(self.theme.metrics.default_column_split_ratio)
    }

    fn render_column(
        &self,
        cx: &mut Context<Self>,
        column_index: usize,
        window_height: f32,
        window_y_offset: f32,
    ) -> Div {
        let column_id = self.column_ids[column_index];
        let top_tab_group = self.top_tab_groups[column_index];
        let bottom_tab_group = self.bottom_tab_groups[column_index];
        let panel_ids = self.panels_by_column[column_index];
        let column_width = self.column_width(column_index);
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let tabs_collapsed = self.column_tabs_collapsed[column_index];

        let header_height = metrics.column_header_height.min(window_height);
        let available_below_header = (window_height - header_height).max(0.0);
        let has_bottom_group = self.column_has_bottom_group[column_index];
        let visible_top_count =
            (if self.panel_slot_visible(column_index, 0) { 1usize } else { 0 })
                + (if self.panel_slot_visible(column_index, 1) { 1usize } else { 0 });
        let visible_bottom_count = if has_bottom_group {
            (if self.panel_slot_visible(column_index, 2) { 1usize } else { 0 })
                + (if self.panel_slot_visible(column_index, 3) { 1usize } else { 0 })
        } else {
            0
        };
        let show_top_group = visible_top_count > 0;
        let show_bottom_group = has_bottom_group && visible_bottom_count > 0;
        let show_group_splitter = show_top_group && show_bottom_group;
        let group_split_hit_height = if show_group_splitter {
            metrics.splitter_hit_zone.max(metrics.splitter_thickness)
                .min(available_below_header)
        } else {
            0.0
        };
        let groups_height = (available_below_header - group_split_hit_height).max(0.0);
        let split_ratio = self.group_split_ratio(column_index).clamp(0.0, 1.0);
        let top_group_height = match (show_top_group, show_bottom_group) {
            (true, true) => groups_height * split_ratio,
            (true, false) => groups_height,
            _ => 0.0,
        };
        let bottom_group_height = match (show_top_group, show_bottom_group) {
            (true, true) => (groups_height - top_group_height).max(0.0),
            (false, true) => groups_height,
            _ => 0.0,
        };
        let top_tab_strip_height = if tabs_collapsed {
            0.0
        } else {
            metrics.tab_strip_height.min(top_group_height)
        };
        let top_content_height = (top_group_height - top_tab_strip_height).max(0.0);
        let bottom_tab_strip_height = if tabs_collapsed {
            0.0
        } else {
            metrics.tab_strip_height.min(bottom_group_height)
        };
        let bottom_content_height = (bottom_group_height - bottom_tab_strip_height).max(0.0);

        let button_size = metrics.button_size.min(column_width);
        let button_gap = metrics.button_gap;
        let content_padding = metrics.content_padding.min(column_width / 2.0);
        let header_spacer_width =
            (column_width - content_padding - button_size * 2.0 - button_gap).max(0.0);
        let tab_rail_width = if tabs_collapsed { button_size } else { 0.0 };
        let column_content_width = (column_width - tab_rail_width).max(0.0);
        let available_tab_width =
            (column_content_width - content_padding * 2.0 - button_size).max(0.0);
        let tab_width = metrics
            .column_tab_width
            .min((available_tab_width / visible_top_count.max(1) as f32).max(0.0));
        let top_tab_spacer_width =
            (available_tab_width - tab_width * visible_top_count as f32).max(0.0);
        let bottom_tab_width = metrics
            .column_tab_width
            .min((available_tab_width / visible_bottom_count.max(1) as f32).max(0.0));
        let bottom_tab_spacer_width =
            (available_tab_width - bottom_tab_width * visible_bottom_count as f32).max(0.0);

        let active_top_panel = self
            .active_panel_id(top_tab_group)
            .filter(|panel_id| self.panel_is_visible(column_index, *panel_id));
        let active_bottom_panel = if has_bottom_group {
            self.active_panel_id(bottom_tab_group)
                .filter(|panel_id| self.panel_is_visible(column_index, *panel_id))
        } else {
            None
        };
        let get_title = |panel_id: PanelId, fallback: &str| {
            self.layout
                .panel_instance(panel_id)
                .map(|panel| self.panels.title(panel))
                .unwrap_or_else(|| fallback.to_owned())
        };
        let column_label = if column_index == 0 { "Left" } else { "Right" };
        let layers_title = format!("{column_label} {}", get_title(panel_ids[0], "Layers"));
        let color_title = format!("{column_label} {}", get_title(panel_ids[1], "Color"));
        let properties_title =
            format!("{column_label} {}", get_title(panel_ids[2], "Properties"));

        let top_content = active_top_panel
            .and_then(|panel_id| self.layout.panel_instance(panel_id))
            .map(|panel| self.panels.render(panel, &self.theme))
            .unwrap_or_else(|| div().into_any_element());
        let bottom_content = active_bottom_panel
            .and_then(|panel_id| self.layout.panel_instance(panel_id))
            .map(|panel| self.panels.render(panel, &self.theme))
            .unwrap_or_else(|| div().into_any_element());

        let layers_active = active_top_panel == Some(panel_ids[0]);
        let color_active = active_top_panel == Some(panel_ids[1]);
        let properties_active = active_bottom_panel == Some(panel_ids[2]);

        let icon_for_panel = |panel_id: PanelId| {
            self.layout
                .panel_instance(panel_id)
                .map(|panel| self.panels.icon(panel).to_owned())
                .unwrap_or_else(|| "◇".to_owned())
        };
        let layers_icon = icon_for_panel(panel_ids[0]);
        let color_icon = icon_for_panel(panel_ids[1]);
        let properties_icon = icon_for_panel(panel_ids[2]);

        let layers_rail_tab = div()
            .id(if column_index == 0 {
                "left-collapsed-tab-layers"
            } else {
                "right-collapsed-tab-layers"
            })
            .flex()
            .items_center()
            .justify_center()
            .w(px(button_size))
            .h(px(button_size))
            .bg(rgb(if layers_active { colors.active_tab } else { colors.dock_background }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if layers_active { colors.primary_text } else { colors.muted_text }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(top_tab_group, panel_ids[0]);
                cx.notify();
            }))
            .child(layers_icon);
        let color_rail_tab = div()
            .id(if column_index == 0 {
                "left-collapsed-tab-color"
            } else {
                "right-collapsed-tab-color"
            })
            .flex()
            .items_center()
            .justify_center()
            .w(px(button_size))
            .h(px(button_size))
            .bg(rgb(if color_active { colors.active_tab } else { colors.dock_background }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if color_active { colors.primary_text } else { colors.muted_text }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(top_tab_group, panel_ids[1]);
                cx.notify();
            }))
            .child(color_icon);
        let properties_rail_tab = div()
            .id(if column_index == 0 {
                "left-collapsed-tab-properties"
            } else {
                "right-collapsed-tab-properties"
            })
            .flex()
            .items_center()
            .justify_center()
            .w(px(button_size))
            .h(px(button_size))
            .bg(rgb(if properties_active { colors.active_tab } else { colors.dock_background }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if properties_active { colors.primary_text } else { colors.muted_text }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(bottom_tab_group, panel_ids[2]);
                cx.notify();
            }))
            .child(properties_icon);

        let layers_tab = div()
            .id(if column_index == 0 { "left-dock-tab-layers" } else { "right-dock-tab-layers" })
            .flex()
            .items_center()
            .justify_center()
            .w(px(tab_width))
            .h(px(top_tab_strip_height))
            .bg(rgb(if layers_active { colors.active_tab } else { colors.dock_background }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if layers_active { colors.primary_text } else { colors.muted_text }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(top_tab_group, panel_ids[0]);
                cx.notify();
            }))
            .child(layers_title);

        let color_tab = div()
            .id(if column_index == 0 { "left-dock-tab-color" } else { "right-dock-tab-color" })
            .flex()
            .items_center()
            .justify_center()
            .w(px(tab_width))
            .h(px(top_tab_strip_height))
            .bg(rgb(if color_active { colors.active_tab } else { colors.dock_background }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if color_active { colors.primary_text } else { colors.muted_text }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(top_tab_group, panel_ids[1]);
                cx.notify();
            }))
            .child(color_title);

        let mut top_tab_strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_content_width))
            .h(px(top_tab_strip_height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)));
        if self.panel_slot_visible(column_index, 0) {
            top_tab_strip = top_tab_strip.child(layers_tab);
        }
        if self.panel_slot_visible(column_index, 1) {
            top_tab_strip = top_tab_strip.child(color_tab);
        }
        if visible_top_count == 0 {
            top_tab_strip = top_tab_strip.child(
                div()
                    .flex()
                    .items_center()
                    .w(px(top_tab_spacer_width))
                    .h(px(top_tab_strip_height))
                    .text_color(rgb(colors.muted_text))
                    .child("No panels"),
            );
        } else {
            top_tab_strip =
                top_tab_strip.child(div().w(px(top_tab_spacer_width)).h(px(1.0)));
        }
        top_tab_strip = top_tab_strip
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_size))
                    .h(px(button_size))
                    .text_color(rgb(colors.primary_text))
                    .bg(rgb(colors.hover))
                    .child("☰"),
            )
            .child(div().w(px(content_padding)).h(px(1.0)));

        let properties_tab = div()
            .id(if column_index == 0 { "left-dock-tab-properties" } else { "right-dock-tab-properties" })
            .flex()
            .items_center()
            .justify_center()
            .w(px(bottom_tab_width))
            .h(px(bottom_tab_strip_height))
            .bg(rgb(if properties_active { colors.active_tab } else { colors.dock_background }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if properties_active { colors.primary_text } else { colors.muted_text }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(bottom_tab_group, panel_ids[2]);
                cx.notify();
            }))
            .child(properties_title);

        let mut bottom_tab_strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_content_width))
            .h(px(bottom_tab_strip_height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)));
        if has_bottom_group && self.panel_slot_visible(column_index, 2) {
            bottom_tab_strip = bottom_tab_strip.child(properties_tab);
        }
        if visible_bottom_count == 0 {
            bottom_tab_strip = bottom_tab_strip.child(
                div()
                    .flex()
                    .items_center()
                    .w(px(bottom_tab_spacer_width))
                    .h(px(bottom_tab_strip_height))
                    .text_color(rgb(colors.muted_text))
                    .child("No panels"),
            );
        } else {
            bottom_tab_strip =
                bottom_tab_strip.child(div().w(px(bottom_tab_spacer_width)).h(px(1.0)));
        }
        bottom_tab_strip = bottom_tab_strip
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_size))
                    .h(px(button_size))
                    .text_color(rgb(colors.primary_text))
                    .bg(rgb(colors.hover))
                    .child("☰"),
            )
            .child(div().w(px(content_padding)).h(px(1.0)));

        let top_content_area = div()
            .w(px(column_content_width))
            .h(px(top_content_height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(top_content);

        let bottom_content_area = div()
            .w(px(column_content_width))
            .h(px(bottom_content_height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(bottom_content);

        let mut top_group = div()
            .flex()
            .flex_col()
            .w(px(column_content_width))
            .h(px(top_group_height));
        if !tabs_collapsed {
            top_group = top_group.child(top_tab_strip);
        }
        top_group = top_group.child(top_content_area);

        let mut bottom_group = div()
            .flex()
            .flex_col()
            .w(px(column_content_width))
            .h(px(bottom_group_height));
        if !tabs_collapsed {
            bottom_group = bottom_group.child(bottom_tab_strip);
        }
        bottom_group = bottom_group.child(bottom_content_area);

        let resize_hit_height = metrics.splitter_hit_zone.max(metrics.splitter_thickness);
        let visible_split_height = metrics.splitter_thickness.min(resize_hit_height);
        let group_splitter_center_y =
            window_y_offset + header_height + top_group_height + group_split_hit_height / 2.0;
        let group_splitter = div()
            .relative()
            .w(px(column_content_width))
            .h(px(group_split_hit_height))
            .cursor(CursorStyle::ResizeUpDown)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.is_resizing_column = false;
                    this.resizing_outer_column = None;
                    this.resizing_groups_column = Some(column_id);
                    this.resize_group_pointer_offset_y =
                        event.position.y.as_f32() - group_splitter_center_y;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(((group_split_hit_height - visible_split_height) / 2.0).max(0.0)))
                    .w(px(column_width))
                    .h(px(visible_split_height))
                    .bg(rgb(colors.splitter)),
            );

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
                    .child(div().w(px(header_spacer_width)).h(px(1.0)))
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
                            .w(px(button_size))
                            .h(px(button_size))
                            .bg(rgb(colors.hover))
                            .text_color(rgb(colors.primary_text))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.column_tabs_collapsed[column_index] =
                                    !this.column_tabs_collapsed[column_index];
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
                            .w(px(button_size))
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
        if show_top_group {
            column_view = column_view.child(top_group);
        }
        if show_group_splitter {
            column_view = column_view.child(group_splitter);
        }
        if show_bottom_group {
            column_view = column_view.child(bottom_group);
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
                .border_l_1()
                .border_color(rgb(colors.border));
            if self.panel_slot_visible(column_index, 0) {
                collapsed_tab_rail = collapsed_tab_rail.child(layers_rail_tab);
            }
            if self.panel_slot_visible(column_index, 1) {
                collapsed_tab_rail = collapsed_tab_rail.child(color_rail_tab);
            }
            if self.panel_slot_visible(column_index, 2) {
                collapsed_tab_rail = collapsed_tab_rail.child(properties_rail_tab);
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

        let left_is_open = self.column_open[0];
        let right_is_open = self.column_open[1];
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

        root.on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
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
                        if let Err(error) = this.layout.apply(
                            DockCommand::ResizeColumn {
                                column: column_id,
                                width: requested_width,
                            },
                            &this.dock_metrics,
                        ) {
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
                        if let Err(error) = this.layout.apply(
                            DockCommand::ResizeColumnBoundary {
                                side: DockSide::Right,
                                boundary_index: 0,
                                delta: delta_x,
                            },
                            &this.dock_metrics,
                        ) {
                            eprintln!("Could not resize adjacent dock columns: {error}");
                            this.is_resizing_column = false;
                        }
                    }
                    cx.notify();
                    return;
                }

                if let Some(column_id) = this.resizing_groups_column {
                    let pointer_y = event.position.y.as_f32();
                    let window_height = window.viewport_size().height.as_f32().max(1.0);
                    let menu_height =
                        this.theme.metrics.column_header_height.min(window_height);
                    let dock_height = (window_height - menu_height).max(1.0);
                    let header_height =
                        this.theme.metrics.column_header_height.min(dock_height);
                    let available_below_header = (dock_height - header_height).max(0.0);
                    let splitter_hit_height = this
                        .theme
                        .metrics
                        .splitter_hit_zone
                        .max(this.theme.metrics.splitter_thickness)
                        .min(available_below_header);
                    let available_height =
                        (available_below_header - splitter_hit_height).max(1.0);
                    let column_index = this
                        .column_ids
                        .iter()
                        .position(|candidate| *candidate == column_id);
                    let Some(column_index) = column_index else {
                        this.resizing_groups_column = None;
                        cx.notify();
                        return;
                    };
                    let current_splitter_center_y = menu_height
                        + header_height
                        + available_height * this.group_split_ratio(column_index).clamp(0.0, 1.0)
                        + splitter_hit_height / 2.0;
                    let target_splitter_center_y =
                        pointer_y - this.resize_group_pointer_offset_y;
                    let delta_y = target_splitter_center_y - current_splitter_center_y;
                    if delta_y != 0.0 {
                        if let Err(error) = this.layout.apply(
                            DockCommand::ResizeGroups {
                                column: column_id,
                                boundary_index: 0,
                                delta: delta_y,
                                available_height,
                            },
                            &this.dock_metrics,
                        ) {
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
                    if this.is_resizing_column
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
