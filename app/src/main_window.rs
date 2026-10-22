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

/// Two adjacent Core-owned columns, each with two vertically stacked tab groups.
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
    top_tab_groups: [NodeId; 2],
    bottom_tab_groups: [NodeId; 2],
    /// Panel IDs per column, ordered: Layers, Color, Properties, History.
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
        let history_key = PanelKey::new("history").expect("built-in panel key must be valid");

        let constraints_for = |key: &PanelKey, title: &str| {
            panels
                .constraints_for_key(key)
                .ok_or_else(|| format!("{title} panel is missing from the panel registry"))
        };
        let layers_constraints = constraints_for(&layers_key, "Layers")?;
        let color_constraints = constraints_for(&color_key, "Color")?;
        let properties_constraints = constraints_for(&properties_key, "Properties")?;
        let history_constraints = constraints_for(&history_key, "History")?;

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
                DockCommand::OpenPanel {
                    key: history_key.clone(),
                    constraints: history_constraints,
                    target: Some(DropTarget::Tabs {
                        group: initial_bottom_group,
                        index: 1,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not add the History tab: {error}"))?;

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

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: color_key.clone(),
                    constraints: color_constraints,
                    target: Some(DropTarget::Tabs {
                        group: left_top_group,
                        index: 1,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not add the left Color tab: {error}"))?;

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: properties_key.clone(),
                    constraints: properties_constraints,
                    target: Some(DropTarget::GroupEdge {
                        group: left_top_group,
                        zone: DropZone::Bottom,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not create the left lower group: {error}"))?;

        let left_bottom_group = second_group_in_column(&layout, column_ids[0])
            .ok_or_else(|| "Dock Core did not create the left lower tab group".to_owned())?;
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: history_key.clone(),
                    constraints: history_constraints,
                    target: Some(DropTarget::Tabs {
                        group: left_bottom_group,
                        index: 1,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not add the left History tab: {error}"))?;

        layout
            .apply(
                DockCommand::RestoreGroupRatios {
                    column: column_ids[0],
                    ratios: vec![split_ratio, 1.0 - split_ratio],
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not apply the left column split ratio: {error}"))?;

        let top_tab_groups = [left_top_group, initial_top_group];
        let bottom_tab_groups = [left_bottom_group, initial_bottom_group];
        let panel_id_for_group = |group: NodeId, key: &PanelKey, title: &str| {
            panel_id_in_group(&layout, group, key)
                .ok_or_else(|| format!("Dock Core did not retain {title} in its tab group"))
        };

        let panels_by_column = [
            [
                panel_id_for_group(left_top_group, &layers_key, "left Layers")?,
                panel_id_for_group(left_top_group, &color_key, "left Color")?,
                panel_id_for_group(left_bottom_group, &properties_key, "left Properties")?,
                panel_id_for_group(left_bottom_group, &history_key, "left History")?,
            ],
            [
                panel_id_for_group(initial_top_group, &layers_key, "right Layers")?,
                panel_id_for_group(initial_top_group, &color_key, "right Color")?,
                panel_id_for_group(initial_bottom_group, &properties_key, "right Properties")?,
                panel_id_for_group(initial_bottom_group, &history_key, "right History")?,
            ],
        ];

        // Both columns start with their first tab active in both groups.
        for column_index in 0..2 {
            layout
                .apply(
                    DockCommand::ActivateTab {
                        group: top_tab_groups[column_index],
                        panel: panels_by_column[column_index][0],
                    },
                    &dock_metrics,
                )
                .map_err(|error| format!("Could not activate Layers in column {column_index}: {error}"))?;
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

        Ok(Self {
            theme,
            panels,
            layout,
            dock_metrics,
            column_ids,
            column_open: [true; 2],
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

    fn panel_is_visible(&self, column_index: usize, panel_id: PanelId) -> bool {
        self.panels_by_column
            .get(column_index)
            .and_then(|panel_ids| panel_ids.iter().position(|candidate| *candidate == panel_id))
            .is_some_and(|panel_index| self.panel_visibility[panel_index])
    }

    fn toggle_panel_visibility(&mut self, panel_index: usize) {
        if panel_index >= self.panel_visibility.len() {
            return;
        }
        self.panel_visibility[panel_index] = !self.panel_visibility[panel_index];

        // Keep Core's active-tab selection aligned with the visible tabs. A tab
        // group can be intentionally empty from the view's perspective.
        for column_index in 0..self.column_ids.len() {
            if !self.column_open[column_index] {
                continue;
            }
            let groups = [
                (self.top_tab_groups[column_index], [0usize, 1usize]),
                (self.bottom_tab_groups[column_index], [2usize, 3usize]),
            ];
            for (group, candidates) in groups {
                let active_visible = self
                    .active_panel_id(group)
                    .is_some_and(|active| self.panel_is_visible(column_index, active));
                if active_visible {
                    continue;
                }
                if let Some(next_index) = candidates
                    .into_iter()
                    .find(|candidate| self.panel_visibility[*candidate])
                {
                    self.activate_tab(group, self.panels_by_column[column_index][next_index]);
                }
            }
        }
    }

    fn close_column(&mut self, column_index: usize) {
        if column_index >= self.column_open.len() || !self.column_open[column_index] {
            return;
        }
        let other_index = 1 - column_index;
        // Keep one dock column alive so the user can still interact with the
        // workspace and use the remaining column's controls.
        if !self.column_open[other_index] {
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

        let header_height = metrics.column_header_height.min(window_height);
        let available_below_header = (window_height - header_height).max(0.0);
        let group_split_hit_height = metrics.splitter_hit_zone.max(metrics.splitter_thickness)
            .min(available_below_header);
        let groups_height = (available_below_header - group_split_hit_height).max(0.0);
        let top_group_height = groups_height * self.group_split_ratio(column_index).clamp(0.0, 1.0);
        let bottom_group_height = (groups_height - top_group_height).max(0.0);
        let top_tab_strip_height = metrics.tab_strip_height.min(top_group_height);
        let top_content_height = (top_group_height - top_tab_strip_height).max(0.0);
        let bottom_tab_strip_height = metrics.tab_strip_height.min(bottom_group_height);
        let bottom_content_height = (bottom_group_height - bottom_tab_strip_height).max(0.0);

        let button_size = metrics.button_size.min(column_width);
        let button_gap = metrics.button_gap;
        let content_padding = metrics.content_padding.min(column_width / 2.0);
        let header_spacer_width =
            (column_width - content_padding - button_size * 2.0 - button_gap).max(0.0);
        let visible_top_count =
            (if self.panel_visibility[0] { 1usize } else { 0 })
                + (if self.panel_visibility[1] { 1usize } else { 0 });
        let visible_bottom_count =
            (if self.panel_visibility[2] { 1usize } else { 0 })
                + (if self.panel_visibility[3] { 1usize } else { 0 });
        let available_tab_width =
            (column_width - content_padding * 2.0 - button_size).max(0.0);
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
        let active_bottom_panel = self
            .active_panel_id(bottom_tab_group)
            .filter(|panel_id| self.panel_is_visible(column_index, *panel_id));
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
        let history_title = format!("{column_label} {}", get_title(panel_ids[3], "History"));

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
        let history_active = active_bottom_panel == Some(panel_ids[3]);

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
            .w(px(column_width))
            .h(px(top_tab_strip_height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)));
        if self.panel_visibility[0] {
            top_tab_strip = top_tab_strip.child(layers_tab);
        }
        if self.panel_visibility[1] {
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

        let history_tab = div()
            .id(if column_index == 0 { "left-dock-tab-history" } else { "right-dock-tab-history" })
            .flex()
            .items_center()
            .justify_center()
            .w(px(bottom_tab_width))
            .h(px(bottom_tab_strip_height))
            .bg(rgb(if history_active { colors.active_tab } else { colors.dock_background }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if history_active { colors.primary_text } else { colors.muted_text }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(bottom_tab_group, panel_ids[3]);
                cx.notify();
            }))
            .child(history_title);

        let mut bottom_tab_strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_width))
            .h(px(bottom_tab_strip_height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)));
        if self.panel_visibility[2] {
            bottom_tab_strip = bottom_tab_strip.child(properties_tab);
        }
        if self.panel_visibility[3] {
            bottom_tab_strip = bottom_tab_strip.child(history_tab);
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
            .w(px(column_width))
            .h(px(top_content_height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(top_content);

        let bottom_content_area = div()
            .w(px(column_width))
            .h(px(bottom_content_height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(bottom_content);

        let top_group = div()
            .flex()
            .flex_col()
            .w(px(column_width))
            .h(px(top_group_height))
            .child(top_tab_strip)
            .child(top_content_area);

        let bottom_group = div()
            .flex()
            .flex_col()
            .w(px(column_width))
            .h(px(bottom_group_height))
            .child(bottom_tab_strip)
            .child(bottom_content_area);

        let resize_hit_height = metrics.splitter_hit_zone.max(metrics.splitter_thickness);
        let visible_split_height = metrics.splitter_thickness.min(resize_hit_height);
        let group_splitter_center_y =
            window_y_offset + header_height + top_group_height + group_split_hit_height / 2.0;
        let group_splitter = div()
            .relative()
            .w(px(column_width))
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

        div()
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
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(button_size))
                            .h(px(button_size))
                            .bg(rgb(colors.hover))
                            .text_color(rgb(colors.primary_text))
                            .child(">>"),
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
            )
            .child(top_group)
            .child(group_splitter)
            .child(bottom_group)
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
        let outer_splitter_width = splitter_width;
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
        let outer_column_splitter = div()
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
            );
        let column_splitter_center_x =
            workspace_width + outer_splitter_width + left_width + splitter_width / 2.0;
        let column_splitter = if both_columns_open {
            let column_splitter = div()
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
                );
    
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
            for (index, (_key, title)) in self.panels.menu_descriptors().into_iter().enumerate().take(4) {
                let visible = self.panel_visibility[index];
                let panel_index = index;
                let item_id = match index {
                    0 => "windows-panel-layers",
                    1 => "windows-panel-color",
                    2 => "windows-panel-properties",
                    _ => "windows-panel-history",
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
