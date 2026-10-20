use dock_core::{
    compute_rects, DockMetrics, DockSide, Layout, LayoutRects, Node, Size as DockSize,
};
use gpui::{Context, Render, Window, div, prelude::*, px, rgb};
use ui::Theme;

use crate::panels::{PanelRegistry, PlaceholderPanelContent};

/// The single main application window. Its tree receives one shared application Theme;
/// child builders borrow that theme rather than constructing their own.
pub struct MainWindow {
    theme: Theme,
    metrics: DockMetrics,
    registry: PanelRegistry,
    layout: Layout,
}

impl MainWindow {
    pub fn new(
        theme: Theme,
        metrics: DockMetrics,
        registry: PanelRegistry,
        layout: Layout,
    ) -> Self {
        Self {
            theme,
            metrics,
            registry,
            layout,
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let typography = self.theme.typography;

        let window_bounds = window.bounds();
        let window_width = window_bounds.size.width.0.max(0.0);
        let window_height = window_bounds.size.height.0.max(0.0);
        let menu_height = metrics.column_header_height.min(window_height);
        let options_height = metrics.tab_strip_height.min((window_height - menu_height).max(0.0));
        let body_height = (window_height - menu_height - options_height).max(0.0);

        // Toolbar is fixed chrome and deliberately excluded from dock-core's geometry.
        // The rest of the body is calculated in logical pixels by the pure core geometry.
        let docking_viewport_width = (window_width - metrics.rail_width).max(0.0);
        let rects = compute_rects(
            &self.layout,
            DockSize::new(docking_viewport_width, body_height),
            &self.metrics,
        );

        let left_dock_width = dock_side_width(&rects, DockSide::Left);
        let right_dock_width = dock_side_width(&rects, DockSide::Right);
        let canvas_width = rects.canvas.size.width.max(0.0);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(colors.window_background))
            .text_color(rgb(colors.primary_text))
            .text_size(px(typography.body_size))
            .font_family(typography.family)
            .child(menu_bar(&self.theme, menu_height))
            .child(options_bar(&self.theme, options_height))
            .child(
                div()
                    .w_full()
                    .h(px(body_height))
                    .flex()
                    .flex_row()
                    .child(toolbar(&self.theme, body_height))
                    .child(render_dock_side(
                        &self.theme,
                        &self.layout,
                        &self.registry,
                        &rects,
                        DockSide::Left,
                        left_dock_width,
                        body_height,
                    ))
                    .child(canvas(&self.theme, canvas_width, body_height))
                    .child(render_dock_side(
                        &self.theme,
                        &self.layout,
                        &self.registry,
                        &rects,
                        DockSide::Right,
                        right_dock_width,
                        body_height,
                    )),
            )
    }
}

fn menu_bar(theme: &Theme, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;
    let labels = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];

    div()
        .w_full()
        .h(px(height))
        .flex()
        .items_center()
        .px(px(metrics.content_padding))
        .bg(rgb(colors.toolbar_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.body_size))
        .children(labels.into_iter().map(|label| {
            div()
                .px(px(metrics.button_gap + metrics.content_padding * 0.5))
                .items_center()
                .child(label)
        }))
}

fn options_bar(theme: &Theme, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    div()
        .w_full()
        .h(px(height))
        .flex()
        .items_center()
        .px(px(metrics.content_padding))
        .bg(rgb(colors.dock_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.tab_size))
        .text_color(rgb(colors.muted_text))
        .child("Options")
}

fn toolbar(theme: &Theme, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;

    // Empty fixed rail: Toolbar is app chrome, not a dock node or a floating panel.
    div()
        .w(px(metrics.rail_width))
        .h(px(height))
        .bg(rgb(colors.toolbar_background))
        .border_1()
        .border_color(rgb(colors.border))
}

fn canvas(theme: &Theme, width: f32, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    div()
        .w(px(width))
        .h(px(height))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(colors.canvas_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.title_size))
        .child(
            div()
                .px(px(metrics.content_padding))
                .text_color(rgb(colors.muted_text))
                .child("Canvas"),
        )
}

fn dock_side_width(rects: &LayoutRects, side: DockSide) -> f32 {
    rects
        .dock_areas
        .iter()
        .filter(|area| area.side == side)
        .map(|area| area.bounds.size.width.max(0.0))
        .sum()
}

fn render_dock_side(
    theme: &Theme,
    layout: &Layout,
    registry: &PanelRegistry,
    rects: &LayoutRects,
    side: DockSide,
    width: f32,
    height: f32,
) -> impl IntoElement {
    let areas = layout
        .dock_areas()
        .iter()
        .filter(|area| area.side == side)
        .map(|area| {
            let area_width = rects
                .dock_areas
                .iter()
                .find(|bounds| bounds.side == side)
                .map(|bounds| bounds.bounds.size.width.max(0.0))
                .unwrap_or_else(|| area.columns.iter().map(|column| column.width.max(0.0)).sum());

            let columns = area.columns.iter().enumerate().map(|(column_index, column)| {
                let column_bounds = rects
                    .columns
                    .iter()
                    .find(|bounds| bounds.side == side && bounds.index == column_index);
                let column_width = column_bounds
                    .map(|bounds| bounds.bounds.size.width.max(0.0))
                    .unwrap_or_else(|| column.width.max(0.0));
                let column_height = column_bounds
                    .map(|bounds| bounds.bounds.size.height.max(0.0))
                    .unwrap_or(height);
                let header_height = rects
                    .column_headers
                    .iter()
                    .find(|bounds| bounds.side == side && bounds.index == column_index)
                    .map(|bounds| bounds.bounds.size.height.max(0.0))
                    .unwrap_or(theme.metrics.column_header_height);

                let group_count = column.groups.len();
                let groups = column.groups.iter().copied().enumerate().map(
                    |(group_index, group_id)| {
                        let group_height = rects
                            .groups
                            .iter()
                            .find(|bounds| bounds.group == group_id)
                            .map(|bounds| bounds.bounds.size.height.max(0.0))
                            .unwrap_or(0.0);
                        let strip_height = rects
                            .tab_strips
                            .iter()
                            .find(|strip| strip.group == group_id)
                            .map(|strip| strip.bounds.size.height.max(0.0))
                            .unwrap_or(theme.metrics.tab_strip_height)
                            .min(group_height);
                        let splitter_height = if group_index + 1 < group_count {
                            theme.metrics.splitter_thickness
                        } else {
                            0.0
                        };

                        div()
                            .w_full()
                            .h(px(group_height + splitter_height))
                            .flex()
                            .flex_col()
                            .child(render_group(
                                theme,
                                layout,
                                registry,
                                group_id,
                                group_height,
                                strip_height,
                            ))
                            .child(
                                div()
                                    .w_full()
                                    .h(px(splitter_height))
                                    .bg(rgb(theme.colors.splitter)),
                            )
                    },
                );

                div()
                    .w(px(column_width))
                    .h(px(column_height))
                    .flex()
                    .flex_col()
                    .child(column_header(theme, header_height))
                    .children(groups)
            });

            div()
                .w(px(area_width))
                .h(px(height))
                .flex()
                .flex_row()
                .children(columns)
        });

    div()
        .w(px(width))
        .h(px(height))
        .flex()
        .flex_row()
        .bg(rgb(theme.colors.dock_background))
        .children(areas)
}

fn column_header(theme: &Theme, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    div()
        .w_full()
        .h(px(height))
        .flex()
        .items_center()
        .px(px(metrics.content_padding))
        .bg(rgb(colors.dock_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.tab_size))
        .text_color(rgb(colors.muted_text))
        .child("Dock")
}

fn render_group(
    theme: &Theme,
    layout: &Layout,
    registry: &PanelRegistry,
    group_id: dock_core::NodeId,
    group_height: f32,
    strip_height: f32,
) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    let mut tab_labels = Vec::new();
    let mut active_content = None;

    if let Some(Node::Tabs(tabs)) = layout.nodes().get(group_id) {
        for (index, panel_id) in tabs.panels.iter().copied().enumerate() {
            let Some(panel) = layout.panels().get(panel_id) else {
                continue;
            };
            let title = registry
                .get(panel.key())
                .map(|descriptor| descriptor.title().to_owned())
                .unwrap_or_else(|| panel.key().as_str().to_owned());
            tab_labels.push((title, index == tabs.active_index));

            if index == tabs.active_index {
                let (heading, message) = registry
                    .create_content(panel.key())
                    .ok()
                    .and_then(|content| {
                        content
                            .as_ref()
                            .as_any()
                            .downcast_ref::<PlaceholderPanelContent>()
                            .map(|placeholder| {
                                (placeholder.heading.clone(), placeholder.message.clone())
                            })
                    })
                    .unwrap_or_else(|| {
                        let title = registry
                            .get(panel.key())
                            .map(|descriptor| descriptor.title().to_owned())
                            .unwrap_or_else(|| panel.key().as_str().to_owned());
                        (title, "Panel placeholder".to_owned())
                    });
                active_content = Some((heading, message));
            }
        }
    }

    let (heading, message) = active_content
        .unwrap_or_else(|| ("Panel".to_owned(), "No panel content".to_owned()));
    let content_height = (group_height - strip_height).max(0.0);
    let tab_row = div()
        .w_full()
        .h(px(strip_height))
        .flex()
        .flex_row()
        .bg(rgb(colors.group_background))
        .border_1()
        .border_color(rgb(colors.border))
        .children(tab_labels.into_iter().map(|(label, active)| {
            let tab_color = if active { colors.active_tab } else { colors.group_background };
            div()
                .flex_1()
                .h(px(strip_height))
                .flex()
                .items_center()
                .px(px(metrics.content_padding))
                .bg(rgb(tab_color))
                .text_size(px(typography.tab_size))
                .child(label)
        }));

    div()
        .w_full()
        .h(px(group_height))
        .flex()
        .flex_col()
        .bg(rgb(colors.group_background))
        .border_1()
        .border_color(rgb(colors.border))
        .child(tab_row)
        .child(
            div()
                .w_full()
                .h(px(content_height))
                .flex()
                .flex_col()
                .px(px(metrics.content_padding))
                .bg(rgb(colors.group_background))
                .child(
                    div()
                        .w_full()
                        .text_size(px(typography.title_size))
                        .child(heading),
                )
                .child(
                    div()
                        .w_full()
                        .text_size(px(typography.tab_size))
                        .text_color(rgb(colors.muted_text))
                        .child(message),
                ),
        )
}
