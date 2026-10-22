use dock_core::{
    DockCommand, DockMetrics, DockSide, DropTarget, Layout, Node, NodeId, PanelId, PanelKey, Profile,
};
use dock_gpui::dock_metrics_from_theme;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::panels::PanelRegistry;

/// Main window with one Core-owned right column and a two-tab group.
pub(crate) struct MainWindow {
    theme: Theme,
    panels: PanelRegistry,
    layout: Layout,
    dock_metrics: DockMetrics,
    tab_group: NodeId,
    layers_panel: PanelId,
    color_panel: PanelId,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let panels = PanelRegistry::default();
        let dock_metrics = dock_metrics_from_theme(&theme);
        let layers_key = PanelKey::new("layers").expect("built-in panel key must be valid");
        let color_key = PanelKey::new("color").expect("built-in panel key must be valid");

        let layers_constraints = panels
            .constraints_for_key(&layers_key)
            .ok_or_else(|| "Layers panel is missing from the panel registry".to_owned())?;
        let color_constraints = panels
            .constraints_for_key(&color_key)
            .ok_or_else(|| "Color panel is missing from the panel registry".to_owned())?;

        // Build the initial composition through Dock Core commands instead of
        // maintaining a separate UI-only list of tabs.
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
        let Node::Column(column) = layout
            .node(right_area.root)
            .ok_or_else(|| "Dock Core returned a missing right column".to_owned())?
        else {
            return Err("Dock Core right dock root is not a column".to_owned());
        };
        let tab_group = *column
            .groups
            .first()
            .ok_or_else(|| "Dock Core did not create a tab group".to_owned())?;

        layout
            .apply(
                DockCommand::OpenPanel {
                    key: color_key.clone(),
                    constraints: color_constraints,
                    target: Some(DropTarget::Tabs {
                        group: tab_group,
                        index: 1,
                    }),
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not add the Color tab: {error}"))?;

        let layers_panel = layout
            .panel_instances()
            .find_map(|(id, panel)| (panel.key == layers_key).then_some(id))
            .ok_or_else(|| "Dock Core did not retain the Layers panel".to_owned())?;
        let color_panel = layout
            .panel_instances()
            .find_map(|(id, panel)| (panel.key == color_key).then_some(id))
            .ok_or_else(|| "Dock Core did not retain the Color panel".to_owned())?;

        // OpenPanel selects the inserted tab. Start with the first tab selected.
        layout
            .apply(
                DockCommand::ActivateTab {
                    group: tab_group,
                    panel: layers_panel,
                },
                &dock_metrics,
            )
            .map_err(|error| format!("Could not activate the Layers tab: {error}"))?;

        Ok(Self {
            theme,
            panels,
            layout,
            dock_metrics,
            tab_group,
            layers_panel,
            color_panel,
        })
    }

    fn activate_tab(&mut self, panel: PanelId) {
        if let Err(error) = self.layout.apply(
            DockCommand::ActivateTab {
                group: self.tab_group,
                panel,
            },
            &self.dock_metrics,
        ) {
            eprintln!("Could not activate dock tab: {error}");
        }
    }

    fn active_panel_id(&self) -> Option<PanelId> {
        let Node::Tabs(tabs) = self.layout.node(self.tab_group)? else {
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
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let window_width = viewport.width.as_f32().max(1.0);
        let window_height = viewport.height.as_f32().max(1.0);

        let colors = self.theme.colors;
        let metrics = self.theme.metrics;

        // The visible column width comes from the Core layout, clamped only to
        // the current viewport so the UI never paints beyond the window edge.
        let column_width = self.column_width().min(window_width);
        let workspace_width = (window_width - column_width).max(0.0);
        let header_height = metrics.column_header_height.min(window_height);
        let tab_strip_height = metrics
            .tab_strip_height
            .min((window_height - header_height).max(0.0));
        let content_height = (window_height - header_height - tab_strip_height).max(0.0);

        let button_size = metrics.button_size.min(column_width);
        let button_gap = metrics.button_gap;
        let content_padding = metrics.content_padding.min(column_width / 2.0);

        let header_spacer_width =
            (column_width - content_padding * 2.0 - button_size * 2.0 - button_gap).max(0.0);
        let tab_width = metrics
            .column_tab_width
            .min(((column_width - content_padding * 2.0 - button_size) / 2.0).max(0.0));
        let tab_spacer_width =
            (column_width - content_padding * 2.0 - tab_width * 2.0 - button_size).max(0.0);

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
        let active_content = self
            .active_panel_id()
            .and_then(|panel_id| self.layout.panel_instance(panel_id))
            .map(|panel| self.panels.render(panel, &self.theme))
            .unwrap_or_else(|| div().into_any_element());
        let layers_active = self.active_panel_id() == Some(self.layers_panel);
        let color_active = self.active_panel_id() == Some(self.color_panel);
        let layers_panel_id = self.layers_panel;
        let color_panel_id = self.color_panel;

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
            .h(px(tab_strip_height))
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
                this.activate_tab(layers_panel_id);
                cx.notify();
            }))
            .child(layers_title);

        let color_tab = div()
            .id("dock-tab-color")
            .flex()
            .items_center()
            .justify_center()
            .w(px(tab_width))
            .h(px(tab_strip_height))
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
                this.activate_tab(color_panel_id);
                cx.notify();
            }))
            .child(color_title);

        let tab_strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_width))
            .h(px(tab_strip_height))
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

        let content = div()
            .w(px(column_width))
            .h(px(content_height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(active_content);

        let right_column = div()
            .flex()
            .flex_col()
            .w(px(column_width))
            .h(px(window_height))
            .bg(rgb(colors.dock_background))
            .child(header)
            .child(tab_strip)
            .child(content);

        div()
            .flex()
            .flex_row()
            .w(px(window_width))
            .h(px(window_height))
            .bg(rgb(colors.canvas_background))
            .child(div().w(px(workspace_width)).h(px(window_height)))
            .child(right_column)
    }
}
