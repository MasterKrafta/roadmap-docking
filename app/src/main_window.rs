use dock_core::{
    DockCommand, DockMetrics, DockSide, Layout, LayoutRects, Node, NodeId, PanelKey,
    Size as DockSize, compute_rects,
};
use dock_gpui::{DockGroupData, TabItem, dock_column_view};
use gpui::{App, Context, Render, Window, div, prelude::*, px, rgb};
use ui::Theme;

use crate::panels::{PanelRegistry, PlaceholderPanelContent};

/// The single main application window. Its tree receives one shared application Theme;
/// child builders borrow that theme rather than constructing their own.
pub struct MainWindow {
    theme: Theme,
    metrics: DockMetrics,
    registry: PanelRegistry,
    layout: Layout,
    window_menu_open: bool,
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
            window_menu_open: false,
        }
    }

    fn toggle_window_menu(&mut self, cx: &mut Context<Self>) {
        self.window_menu_open = !self.window_menu_open;
        cx.notify();
    }

    fn open_or_activate_panel(&mut self, key: PanelKey, cx: &mut Context<Self>) {
        // The menu represents registered panel types, not instances. Prefer an existing
        // instance in either docked or floating layout so even non-singleton descriptors
        // do not produce accidental duplicates when selected from Window > Panels.
        let existing_panel = self.layout.panels().iter().find_map(|(panel_id, panel)| {
            (panel.key() == &key).then_some(panel_id)
        });

        let result: Result<(), String> = if let Some(panel_id) = existing_panel {
            let dock_location = self.layout.nodes().iter().find_map(|(group, node)| {
                match node {
                    Node::Tabs(tabs) => tabs
                        .panels
                        .iter()
                        .position(|candidate| *candidate == panel_id)
                        .map(|index| (group, index)),
                    Node::Split(_) => None,
                }
            });

            if let Some((group, index)) = dock_location {
                let already_active = matches!(
                    self.layout.nodes().get(group),
                    Some(Node::Tabs(tabs)) if tabs.active_index == index
                );
                if already_active {
                    Ok(())
                } else {
                    self.layout
                        .apply(DockCommand::ActivateTab { group, index }, &self.metrics)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }
            } else if let Some((floating, index)) =
                self.layout.floating_groups().iter().find_map(|(floating, group)| {
                    group
                        .panels
                        .iter()
                        .position(|candidate| *candidate == panel_id)
                        .map(|index| (floating, index))
                })
            {
                self.layout
                    .apply(
                        DockCommand::ActivateFloatingTab { floating, index },
                        &self.metrics,
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string())
                    .and_then(|()| {
                        self.layout
                            .apply(DockCommand::RaiseFloating { floating }, &self.metrics)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            } else {
                Err(format!("panel instance {panel_id:?} is not placed in the layout"))
            }
        } else {
            self.registry
                .open_command(&key, None)
                .map_err(|error| error.to_string())
                .and_then(|command| {
                    self.layout
                        .apply(command, &self.metrics)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                })
        };

        match result {
            Ok(()) => {
                self.window_menu_open = false;
                cx.notify();
            }
            Err(error) => eprintln!("Could not open or activate panel {key:?}: {error}"),
        }
    }

    fn toggle_panel_from_menu(&mut self, key: PanelKey, cx: &mut Context<Self>) {
        let existing_panel = self.layout.panels().iter().find_map(|(panel_id, panel)| {
            (panel.key() == &key).then_some(panel_id)
        });

        if let Some(panel_id) = existing_panel {
            match self.layout.apply(DockCommand::ClosePanel { panel: panel_id }, &self.metrics) {
                Ok(_) => {
                    self.window_menu_open = false;
                    cx.notify();
                }
                Err(error) => eprintln!("Could not close panel {key:?} from Window menu: {error}"),
            }
        } else {
            self.open_or_activate_panel(key, cx);
        }
    }

    fn activate_tab(
        &mut self,
        group_id: NodeId,
        tab_index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(Node::Tabs(tabs)) = self.layout.nodes().get(group_id) {
            if tabs.active_index == tab_index {
                return;
            }
        }

        match self.layout.apply(
            DockCommand::ActivateTab {
                group: group_id,
                index: tab_index,
            },
            &self.metrics,
        ) {
            Ok(_) => cx.notify(),
            Err(error) => eprintln!(
                "Could not activate tab {tab_index} in group {group_id:?}: {error}"
            ),
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let typography = self.theme.typography;
        let weak_window = cx.entity().downgrade();

        let toggle_menu_window = weak_window.clone();
        let on_toggle_window_menu = move |app: &mut App| {
            if let Err(error) = toggle_menu_window.update(app, |this, cx| {
                this.toggle_window_menu(cx);
            }) {
                eprintln!("Could not dispatch Window menu toggle: {error}");
            }
        };

        let open_panel_window = weak_window.clone();
        let on_open_panel = move |key: PanelKey, app: &mut App| {
            if let Err(error) = open_panel_window.update(app, |this, cx| {
                this.toggle_panel_from_menu(key.clone(), cx);
            }) {
                eprintln!("Could not dispatch panel open: {error}");
            }
        };

        let activate_tab_window = weak_window.clone();
        let on_activate_tab = move |group_id, tab_index, app: &mut App| {
            if let Err(error) = activate_tab_window.update(app, |this, cx| {
                this.activate_tab(group_id, tab_index, cx);
            }) {
                eprintln!("Could not dispatch tab activation: {error}");
            }
        };

        let window_bounds = window.bounds();
        let window_width = window_bounds.size.width.as_f32().max(0.0);
        let window_height = window_bounds.size.height.as_f32().max(0.0);
        let menu_height = metrics.column_header_height.min(window_height);
        let panel_menu_height = if self.window_menu_open {
            (metrics.tab_strip_height.max(24.0) * (self.registry.len() as f32 + 1.0)
                + metrics.content_padding
                + 2.0)
                .min((window_height - menu_height - metrics.tab_strip_height).max(0.0))
        } else {
            0.0
        };
        let options_height = metrics
            .tab_strip_height
            .min((window_height - menu_height - panel_menu_height).max(0.0));
        let body_height =
            (window_height - menu_height - panel_menu_height - options_height).max(0.0);

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
            .child(menu_bar(
                &self.theme,
                menu_height,
                on_toggle_window_menu,
            ))
            .when(self.window_menu_open, |element| {
                element.child(panels_menu(
                    &self.theme,
                    &self.registry,
                    &self.layout,
                    panel_menu_height,
                    on_open_panel,
                ))
            })
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
                        on_activate_tab.clone(),
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
                        on_activate_tab,
                    )),
            )
    }
}

fn menu_bar<OnWindowClick>(
    theme: &Theme,
    height: f32,
    on_window_click: OnWindowClick,
) -> impl IntoElement
where
    OnWindowClick: Fn(&mut App) + Clone + 'static,
{
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
            let click = on_window_click.clone();
            let is_window_menu = label == "Window";
            div()
                .id(format!("menu-item-{label}"))
                .h(px(height))
                .px(px(metrics.button_gap + metrics.content_padding * 0.5))
                .items_center()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(colors.hover)))
                .on_click(move |_, _, app| {
                    if is_window_menu {
                        click(app);
                    }
                })
                .child(label)
        }))
}

fn panels_menu<OnOpen>(
    theme: &Theme,
    registry: &PanelRegistry,
    layout: &Layout,
    height: f32,
    on_open: OnOpen,
) -> impl IntoElement
where
    OnOpen: Fn(PanelKey, &mut App) + Clone + 'static,
{
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;
    let row_height = metrics.tab_strip_height.max(24.0);

    let rows = registry.descriptors().into_iter().map(|descriptor| {
        let key = descriptor.key().clone();
        let title = descriptor.title().to_owned();
        let is_open = panel_is_open(layout, &key);

        let open_key = key.clone();
        let open = on_open.clone();

        div()
            .id(format!("window-panel-{}", key.as_str()))
            .w_full()
            .h(px(row_height))
            .flex()
            .flex_row()
            .items_center()
            .px(px(metrics.content_padding))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(colors.hover)))
            .on_click(move |_, _, app| open(open_key.clone(), app))
            .child(
                div()
                    .w(px(20.0))
                    .h(px(row_height))
                    .flex()
                    .items_center()
                    .text_color(rgb(colors.primary_text))
                    .child(if is_open { "✓" } else { "" }),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(row_height))
                    .flex()
                    .items_center()
                    .child(title),
            )
    });

    div()
        .w(px(280.0))
        .h(px(height))
        .flex()
        .flex_col()
        .overflow_hidden()
        .py(px(metrics.content_padding * 0.5))
        .bg(rgb(colors.group_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.body_size))
        .text_color(rgb(colors.primary_text))
        .child(
            div()
                .h(px(row_height))
                .px(px(metrics.content_padding))
                .flex()
                .items_center()
                .text_size(px(typography.tab_size))
                .text_color(rgb(colors.muted_text))
                .child("Panels"),
        )
        .children(rows)
}

fn panel_is_open(layout: &Layout, key: &PanelKey) -> bool {
    layout.panels().values().any(|panel| panel.key() == key)
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

fn render_dock_side<OnActivate>(
    theme: &Theme,
    layout: &Layout,
    registry: &PanelRegistry,
    rects: &LayoutRects,
    side: DockSide,
    width: f32,
    height: f32,
    on_activate: OnActivate,
) -> impl IntoElement
where
    OnActivate: Fn(NodeId, usize, &mut App) + Clone + 'static,
{
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
                    on_activate.clone(),
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
        group_id,
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
    fn active_panel_projection_tracks_each_groups_independent_layout_state() {
        use dock_core::{DockCommand, Node};

        let theme = Theme::default();
        let metrics = dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let mut layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");
        let groups = layout.dock_areas()[0].columns[0].groups.clone();

        assert_eq!(groups.len(), 2);

        let upper_initial = group_view_data(
            &layout,
            &registry,
            groups[0],
            220.0,
            theme.metrics.tab_strip_height,
        );
        assert_eq!(
            upper_initial.tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>(),
            vec!["Layers", "Color"]
        );
        assert!(upper_initial.tabs[0].active);
        assert!(!upper_initial.tabs[1].active);
        assert_eq!(upper_initial.active_heading, "Layers");
        assert_eq!(
            upper_initial.active_message,
            "Layer stack and visibility controls."
        );

        let lower_initial = group_view_data(
            &layout,
            &registry,
            groups[1],
            220.0,
            theme.metrics.tab_strip_height,
        );
        assert_eq!(
            lower_initial.tabs.iter().map(|tab| tab.title.as_str()).collect::<Vec<_>>(),
            vec!["Properties", "History"]
        );
        assert!(lower_initial.tabs[0].active);
        assert_eq!(lower_initial.active_heading, "Properties");

        let original_version = layout.version();
        layout
            .apply(
                DockCommand::ActivateTab {
                    group: groups[0],
                    index: 1,
                },
                &metrics,
            )
            .expect("the Color tab can be activated");
        assert!(layout.version() > original_version);

        let upper_after_activation = group_view_data(
            &layout,
            &registry,
            groups[0],
            220.0,
            theme.metrics.tab_strip_height,
        );
        let lower_after_activation = group_view_data(
            &layout,
            &registry,
            groups[1],
            220.0,
            theme.metrics.tab_strip_height,
        );

        assert!(!upper_after_activation.tabs[0].active);
        assert!(upper_after_activation.tabs[1].active);
        assert_eq!(upper_after_activation.active_heading, "Color");
        assert_eq!(
            upper_after_activation.active_message,
            "Color selection and foreground/background swatches."
        );
        assert!(lower_after_activation.tabs[0].active);
        assert!(!lower_after_activation.tabs[1].active);
        assert_eq!(lower_after_activation.active_heading, "Properties");

        layout
            .apply(
                DockCommand::ActivateTab {
                    group: groups[1],
                    index: 1,
                },
                &metrics,
            )
            .expect("the History tab can be activated");

        let upper_after_second_activation = group_view_data(
            &layout,
            &registry,
            groups[0],
            220.0,
            theme.metrics.tab_strip_height,
        );
        let lower_after_second_activation = group_view_data(
            &layout,
            &registry,
            groups[1],
            220.0,
            theme.metrics.tab_strip_height,
        );

        assert!(upper_after_second_activation.tabs[1].active);
        assert_eq!(upper_after_second_activation.active_heading, "Color");
        assert!(!lower_after_second_activation.tabs[0].active);
        assert!(lower_after_second_activation.tabs[1].active);
        assert_eq!(lower_after_second_activation.active_heading, "History");
        assert_eq!(
            lower_after_second_activation.active_message,
            "Recent editing operations."
        );
    }
    #[test]
    fn panel_menu_commands_close_reopen_and_activate_without_duplicates() {
        use dock_core::{DockCommand, Node, PanelKey};

        let theme = Theme::default();
        let metrics = dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let mut layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");
        let initial_groups = layout.dock_areas()[0].columns[0].groups.clone();
        let color_key = PanelKey::new("color").expect("valid panel key");

        let color_panel = layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| (panel.key() == &color_key).then_some(panel_id))
            .expect("Color starts open");
        layout
            .apply(DockCommand::ClosePanel { panel: color_panel }, &metrics)
            .expect("ClosePanel should close Color");
        assert_eq!(layout.panels().len(), 3);
        assert!(!layout.panels().iter().any(|(_, panel)| panel.key() == &color_key));

        let open_command = registry
            .open_command(&color_key, None)
            .expect("registered Color has an open command");
        layout
            .apply(open_command, &metrics)
            .expect("OpenPanel should reopen Color");
        assert_eq!(layout.panels().len(), 4);

        let color_panel = layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| (panel.key() == &color_key).then_some(panel_id))
            .expect("Color should exist after reopening");
        let (group_id, tab_index, active_index) = layout
            .nodes()
            .iter()
            .find_map(|(group_id, node)| match node {
                Node::Tabs(tabs) => tabs
                    .panels
                    .iter()
                    .position(|panel_id| *panel_id == color_panel)
                    .map(|index| (group_id, index, tabs.active_index)),
                Node::Split(_) => None,
            })
            .expect("reopened Color should belong to a tab group");
        assert_eq!(group_id, initial_groups[0]);
        assert_eq!(tab_index, 1);
        assert_eq!(active_index, 1);

        // OpenPanel remains safe if another caller requests an already-open singleton.
        let activate_existing = registry
            .open_command(&color_key, None)
            .expect("registered Color has an open command");
        layout
            .apply(activate_existing, &metrics)
            .expect("OpenPanel should activate existing Color");
        assert_eq!(layout.panels().len(), 4);
    }


    #[test]
    fn window_panel_menu_marks_registered_open_panels_and_excludes_canvas() {
        use super::panel_is_open;
        use dock_core::{DockCommand, PanelKey};

        let theme = Theme::default();
        let metrics = crate::dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let mut layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");

        assert!(
            registry
                .descriptors()
                .iter()
                .all(|descriptor| descriptor.key().as_str() != "canvas"),
            "Canvas is window chrome and must not appear in the registered-panel menu"
        );

        let layers_key = PanelKey::new("layers").expect("built-in key is valid");
        let color_key = PanelKey::new("color").expect("built-in key is valid");
        assert!(panel_is_open(&layout, &layers_key));
        assert!(panel_is_open(&layout, &color_key));

        let color_panel = layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| {
                (panel.key() == &color_key).then_some(panel_id)
            })
            .expect("Color panel is open");
        layout
            .apply(DockCommand::ClosePanel { panel: color_panel }, &metrics)
            .expect("an ordinary panel can be closed");

        assert!(panel_is_open(&layout, &layers_key));
        assert!(!panel_is_open(&layout, &color_key));
    }

}