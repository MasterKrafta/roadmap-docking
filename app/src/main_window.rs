use dock_core::{
    DockCommand, DockMetrics, DockSide, DropTarget, DropZone, Layout, Node, NodeId, PanelId,
    PanelKey, Profile,
};
use dock_gpui::dock_metrics_from_theme;
use gpui::{
    Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
    rgb,
};
use ui::Theme;

use crate::panels::PanelRegistry;

/// Main window with one Core-owned right column and two vertically stacked tab groups.
pub(crate) struct MainWindow {
    theme: Theme,
    panels: PanelRegistry,
    layout: Layout,
    dock_metrics: DockMetrics,
    column_id: NodeId,
    top_tab_group: NodeId,
    bottom_tab_group: NodeId,
    layers_panel: PanelId,
    color_panel: PanelId,
    properties_panel: PanelId,
    history_panel: PanelId,
    is_resizing_column: bool,
    resize_start_x: f32,
    resize_start_width: f32,
    is_resizing_groups: bool,
    resize_group_last_y: f32,
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

        // Create both groups and their tabs through Dock Core commands. The
        // registry owns panel presentation; Core owns placement and active tabs.
        let mut layout = Layout::new(Profile::Photoshop);
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

        let right_area = layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Dock Core did not create the right dock area".to_owned())?;
        let column_id = right_area.root;
        let Node::Column(column) = layout
            .node(column_id)
            .ok_or_else(|| "Dock Core returned a missing right column".to_owned())?
        else {
            return Err("Dock Core right dock root is not a column".to_owned());
        };
        let top_tab_group = *column
            .groups
            .first()
            .ok_or_else(|| "Dock Core did not create the upper tab group".to_owned())?;

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: color_key.clone(),
                    constraints: color_constraints,
                    target: Some(DropTarget::Tabs {
                        group: top_tab_group,
                        index: 1,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not add the Color tab: {error}"))?;

        // Opening a panel at the bottom edge creates a second Tabs group in the
        // same Core column, with the provisional 50/50 split corrected to Theme's ratio below.
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: properties_key.clone(),
                    constraints: properties_constraints,
                    target: Some(DropTarget::GroupEdge {
                        group: top_tab_group,
                        zone: DropZone::Bottom,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not create the lower tab group: {error}"))?;

        let Node::Column(column) = layout
            .node(column_id)
            .ok_or_else(|| "Dock Core lost the right column after splitting it".to_owned())?
        else {
            return Err("Dock Core right dock root changed from a column".to_owned());
        };
        let bottom_tab_group = *column
            .groups
            .get(1)
            .ok_or_else(|| "Dock Core did not create the lower tab group".to_owned())?;

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: history_key.clone(),
                    constraints: history_constraints,
                    target: Some(DropTarget::Tabs {
                        group: bottom_tab_group,
                        index: 1,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not add the History tab: {error}"))?;

        layout
            .apply(
                DockCommand::RestoreGroupRatios {
                    column: column_id,
                    ratios: vec![split_ratio, 1.0 - split_ratio],
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not apply the Theme group split ratio: {error}"))?;

        let panel_id_for = |key: &PanelKey, title: &str| {
            layout
                .panel_instances()
                .find_map(|(id, panel)| (panel.key == *key).then_some(id))
                .ok_or_else(|| format!("Dock Core did not retain the {title} panel"))
        };
        let layers_panel = panel_id_for(&layers_key, "Layers")?;
        let color_panel = panel_id_for(&color_key, "Color")?;
        let properties_panel = panel_id_for(&properties_key, "Properties")?;
        let history_panel = panel_id_for(&history_key, "History")?;

        layout
            .apply(
                DockCommand::ActivateTab {
                    group: top_tab_group,
                    panel: layers_panel,
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not activate the Layers tab: {error}"))?;
        layout
            .apply(
                DockCommand::ActivateTab {
                    group: bottom_tab_group,
                    panel: properties_panel,
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not activate the Properties tab: {error}"))?;

        Ok(Self {
            theme,
            panels,
            layout,
            dock_metrics,
            column_id,
            top_tab_group,
            bottom_tab_group,
            layers_panel,
            color_panel,
            properties_panel,
            history_panel,
            is_resizing_column: false,
            resize_start_x: 0.0,
            resize_start_width: dock_metrics.default_column_width,
            is_resizing_groups: false,
            resize_group_last_y: 0.0,
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

    fn column_width(&self) -> f32 {
        self.layout
            .main_area(DockSide::Right)
            .and_then(|area| self.layout.node(area.root))
            .and_then(|node| match node {
                Node::Column(column) => Some(column.width),
                _ => None,
            })
            .unwrap_or(self.theme.metrics.default_column_width)
    }

    fn group_split_ratio(&self) -> f32 {
        self.layout
            .node(self.column_id)
            .and_then(|node| match node {
                Node::Column(column) => column.group_ratios.first().copied(),
                _ => None,
            })
            .unwrap_or(self.theme.metrics.default_column_split_ratio)
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let window_width = viewport.width.as_f32().max(1.0);
        let window_height = viewport.height.as_f32().max(1.0);

        let colors = self.theme.colors;
        let metrics = self.theme.metrics;

        let column_width = self.column_width().min(window_width);
        let workspace_width = (window_width - column_width).max(0.0);
        let header_height = metrics.column_header_height.min(window_height);
        let available_below_header = (window_height - header_height).max(0.0);
        let splitter_height = metrics.splitter_thickness.min(available_below_header);
        let groups_height = (available_below_header - splitter_height).max(0.0);
        let top_group_height = groups_height * self.group_split_ratio().clamp(0.0, 1.0);
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
        let tab_width = metrics
            .column_tab_width
            .min(((column_width - content_padding * 2.0 - button_size) / 2.0).max(0.0));
        let tab_spacer_width =
            (column_width - content_padding * 2.0 - tab_width * 2.0 - button_size).max(0.0);

        let top_active_panel = self.active_panel_id(self.top_tab_group);
        let bottom_active_panel = self.active_panel_id(self.bottom_tab_group);

        let layers_title = self
            .layout
            .panel_instance(self.layers_panel)
            .map(|panel| self.panels.title(panel))
            .unwrap_or_else(|| "Layers".to_owned());
        let color_title = self
            .layout
            .panel_instance(self.color_panel)
            .map(|panel| self.panels.title(panel))
            .unwrap_or_else(|| "Color".to_owned());
        let properties_title = self
            .layout
            .panel_instance(self.properties_panel)
            .map(|panel| self.panels.title(panel))
            .unwrap_or_else(|| "Properties".to_owned());
        let history_title = self
            .layout
            .panel_instance(self.history_panel)
            .map(|panel| self.panels.title(panel))
            .unwrap_or_else(|| "History".to_owned());

        let top_content = top_active_panel
            .and_then(|panel_id| self.layout.panel_instance(panel_id))
            .map(|panel| self.panels.render(panel, &self.theme))
            .unwrap_or_else(|| div().into_any_element());
        let bottom_content = bottom_active_panel
            .and_then(|panel_id| self.layout.panel_instance(panel_id))
            .map(|panel| self.panels.render(panel, &self.theme))
            .unwrap_or_else(|| div().into_any_element());

        let layers_active = top_active_panel == Some(self.layers_panel);
        let color_active = top_active_panel == Some(self.color_panel);
        let properties_active = bottom_active_panel == Some(self.properties_panel);
        let history_active = bottom_active_panel == Some(self.history_panel);

        let top_tab_group = self.top_tab_group;
        let bottom_tab_group = self.bottom_tab_group;
        let layers_panel_id = self.layers_panel;
        let color_panel_id = self.color_panel;
        let properties_panel_id = self.properties_panel;
        let history_panel_id = self.history_panel;

        let header = div()
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
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_size))
                    .h(px(button_size))
                    .bg(rgb(colors.hover))
                    .text_color(rgb(colors.primary_text))
                    .child("×"),
            )
            .child(div().w(px(content_padding)).h(px(1.0)));

        let layers_tab = div()
            .id("dock-tab-layers")
            .flex()
            .items_center()
            .justify_center()
            .w(px(tab_width))
            .h(px(top_tab_strip_height))
            .bg(rgb(if layers_active {
                colors.active_tab
            } else {
                colors.dock_background
            }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if layers_active {
                colors.primary_text
            } else {
                colors.muted_text
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(top_tab_group, layers_panel_id);
                cx.notify();
            }))
            .child(layers_title);

        let color_tab = div()
            .id("dock-tab-color")
            .flex()
            .items_center()
            .justify_center()
            .w(px(tab_width))
            .h(px(top_tab_strip_height))
            .bg(rgb(if color_active {
                colors.active_tab
            } else {
                colors.dock_background
            }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if color_active {
                colors.primary_text
            } else {
                colors.muted_text
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(top_tab_group, color_panel_id);
                cx.notify();
            }))
            .child(color_title);

        let top_tab_strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_width))
            .h(px(top_tab_strip_height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)))
            .child(layers_tab)
            .child(color_tab)
            .child(div().w(px(tab_spacer_width)).h(px(1.0)))
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
            .id("dock-tab-properties")
            .flex()
            .items_center()
            .justify_center()
            .w(px(tab_width))
            .h(px(bottom_tab_strip_height))
            .bg(rgb(if properties_active {
                colors.active_tab
            } else {
                colors.dock_background
            }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if properties_active {
                colors.primary_text
            } else {
                colors.muted_text
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(bottom_tab_group, properties_panel_id);
                cx.notify();
            }))
            .child(properties_title);

        let history_tab = div()
            .id("dock-tab-history")
            .flex()
            .items_center()
            .justify_center()
            .w(px(tab_width))
            .h(px(bottom_tab_strip_height))
            .bg(rgb(if history_active {
                colors.active_tab
            } else {
                colors.dock_background
            }))
            .border_1()
            .border_color(rgb(colors.border))
            .text_color(rgb(if history_active {
                colors.primary_text
            } else {
                colors.muted_text
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_tab(bottom_tab_group, history_panel_id);
                cx.notify();
            }))
            .child(history_title);

        let bottom_tab_strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_width))
            .h(px(bottom_tab_strip_height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)))
            .child(properties_tab)
            .child(history_tab)
            .child(div().w(px(tab_spacer_width)).h(px(1.0)))
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

        let splitter = div()
            .w(px(column_width))
            .h(px(splitter_height))
            .bg(rgb(colors.splitter))
            .cursor(CursorStyle::ResizeUpDown)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.is_resizing_column = false;
                    this.is_resizing_groups = true;
                    this.resize_group_last_y = event.position.y.as_f32();
                    cx.notify();
                }),
            );

        // The free, canvas-facing edge of the right dock column owns this
        // resize handle. Width changes are committed through Dock Core.
        let resize_hit_zone = metrics.splitter_hit_zone;
        let resize_line = metrics.splitter_thickness.min(resize_hit_zone);
        let resize_handle = div()
            .absolute()
            .left(px(-resize_hit_zone / 2.0))
            .top(px(0.0))
            .w(px(resize_hit_zone))
            .h(px(window_height))
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.resize_start_width = this.column_width();
                    this.resize_start_x = event.position.x.as_f32();
                    this.is_resizing_column = true;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px((resize_hit_zone - resize_line) / 2.0))
                    .top(px(0.0))
                    .w(px(resize_line))
                    .h(px(window_height))
                    .bg(rgb(colors.splitter)),
            );

        let right_column = div()
            .relative()
            .flex()
            .flex_col()
            .w(px(column_width))
            .h(px(window_height))
            .bg(rgb(colors.dock_background))
            .child(header)
            .child(top_group)
            .child(splitter)
            .child(bottom_group)
            .child(resize_handle);

        div()
            .flex()
            .flex_row()
            .w(px(window_width))
            .h(px(window_height))
            .bg(rgb(colors.canvas_background))
            .child(div().w(px(workspace_width)).h(px(window_height)))
            .child(right_column)
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.is_resizing_groups {
                    let pointer_y = event.position.y.as_f32();
                    let delta_y = pointer_y - this.resize_group_last_y;
                    this.resize_group_last_y = pointer_y;

                    if delta_y != 0.0 {
                        let viewport_height = window.viewport_size().height.as_f32().max(1.0);
                        let available_height = (viewport_height
                            - this.theme.metrics.column_header_height
                            - this.theme.metrics.splitter_thickness)
                            .max(1.0);
                        if let Err(error) = this.layout.apply(
                            DockCommand::ResizeGroups {
                                column: this.column_id,
                                boundary_index: 0,
                                delta: delta_y,
                                available_height,
                            },
                            &this.dock_metrics,
                        ) {
                            eprintln!("Could not resize dock groups: {error}");
                            this.is_resizing_groups = false;
                        }
                    }
                    cx.notify();
                    return;
                }

                if !this.is_resizing_column {
                    return;
                }

                let delta_x = event.position.x.as_f32() - this.resize_start_x;
                let minimum_width = this.dock_metrics.min_column_width;
                let viewport_width = window.viewport_size().width.as_f32().max(1.0);
                let maximum_width =
                    (viewport_width - this.theme.metrics.min_document_width).max(minimum_width);
                let requested_width =
                    (this.resize_start_width - delta_x).clamp(minimum_width, maximum_width);

                if let Err(error) = this.layout.apply(
                    DockCommand::ResizeColumn {
                        column: this.column_id,
                        width: requested_width,
                    },
                    &this.dock_metrics,
                ) {
                    eprintln!("Could not resize dock column: {error}");
                    this.is_resizing_column = false;
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    if this.is_resizing_column || this.is_resizing_groups {
                        this.is_resizing_column = false;
                        this.is_resizing_groups = false;
                        cx.notify();
                    }
                }),
            )
    }
}
