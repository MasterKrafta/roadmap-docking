use dock_core::{
    DockMetrics, DockSide, Layout, LayoutRects, Node, Size as DockSize, compute_rects,
};
use dock_gpui::{DockGroupData, TabItem, dock_column_view};
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
        let window_width = window_bounds.size.width.as_f32().max(0.0);
        let window_height = window_bounds.size.height.as_f32().max(0.0);
        let menu_height = metrics.column_header_height.min(window_height);
        let options_height = metrics
            .tab_strip_height
            .min((window_height - menu_height).max(0.0));
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
    let labels = [
        "File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help",
    ];

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
                .unwrap_or_else(|| {
                    area.columns
                        .iter()
                        .map(|column| column.width.max(0.0))
                        .sum()
                });

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

                let groups = column.groups.iter().copied().map(|group_id| {
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

                    group_view_data(layout, registry, group_id, group_height, strip_height)
                }).collect::<Vec<_>>();

                dock_column_view(
                    theme,
                    column_width,
                    column_height,
                    header_height,
                    groups,
                )
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

fn group_view_data(
    layout: &Layout,
    registry: &PanelRegistry,
    group_id: dock_core::NodeId,
    group_height: f32,
    tab_strip_height: f32,
) -> DockGroupData {
    let mut tabs = Vec::new();
    let mut active_content = None;

    if let Some(Node::Tabs(group)) = layout.nodes().get(group_id) {
        for (index, panel_id) in group.panels.iter().copied().enumerate() {
            let Some(panel) = layout.panels().get(panel_id) else {
                continue;
            };

            let title = registry
                .get(panel.key())
                .map(|descriptor| descriptor.title().to_owned())
                .unwrap_or_else(|| panel.key().as_str().to_owned());

            if index == group.active_index {
                let fallback = (title.clone(), "Panel placeholder".to_owned());
                active_content = registry
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
                    .or(Some(fallback));
            }

            tabs.push(TabItem {
                title,
                active: index == group.active_index,
            });
        }
    }

    let (active_heading, active_message) =
        active_content.unwrap_or_else(|| ("Panel".to_owned(), "No panel content".to_owned()));

    DockGroupData {
        height: group_height,
        tab_strip_height,
        tabs,
        active_heading,
        active_message,
    }
}

#[cfg(test)]
mod tests {
    use super::group_view_data;
    use crate::{dock_metrics, panels::PanelRegistry};
    use ui::Theme;

    #[test]
    fn static_group_projection_uses_layout_tabs_and_registered_active_content() {
        let theme = Theme::default();
        let metrics = dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");
        let groups = &layout.dock_areas()[0].columns[0].groups;

        assert_eq!(groups.len(), 2);

        let upper = group_view_data(&layout, &registry, groups[0], 220.0, theme.metrics.tab_strip_height);
        assert_eq!(
            upper.tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>(),
            vec!["Layers", "Color"]
        );
        assert!(upper.tabs[0].active);
        assert!(!upper.tabs[1].active);
        assert_eq!(upper.active_heading, "Layers");
        assert_eq!(upper.active_message, "Layer stack and visibility controls.");

        let lower = group_view_data(&layout, &registry, groups[1], 220.0, theme.metrics.tab_strip_height);
        assert_eq!(
            lower.tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>(),
            vec!["Properties", "History"]
        );
        assert!(lower.tabs[0].active);
        assert_eq!(lower.active_heading, "Properties");
    }
}
