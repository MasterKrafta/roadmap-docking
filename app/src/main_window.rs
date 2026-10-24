mod column_tabs_group;
mod tabs_group;

use dock_core::{
    create_initial_layout, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, Layout,
    Node, NodeId, PanelConstraints, PanelId, PanelKey, Point, Profile, Rect, Size,
};
use gpui::{
    Context, CursorStyle, DragMoveEvent, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

#[derive(Clone, Copy)]
struct GroupTabsView {
    active_tab: usize,
    first_panel: PanelId,
    first_label: &'static str,
    second_panel: Option<PanelId>,
    second_label: Option<&'static str>,
}

/// Snapshot captured when a splitter drag begins. Each pointer update is applied
/// to this same baseline, so the total pointer delta is not accumulated repeatedly.
#[derive(Clone)]
struct SplitterDragState {
    layout_before: Layout,
    mouse_position_before: gpui::Point<gpui::Pixels>,
    owner: NodeId,
    boundary_index: usize,
    axis: dock_core::Axis,
    available_height: f32,
}

/// GPUI requires a view entity for a drag; resizing itself is handled by the Core layout.
struct SplitterDragPreview;

impl Render for SplitterDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Root application window.
pub(crate) struct MainWindow {
    theme: ui::Theme,
    dockers_menu_open: bool,
    checked_panels: [bool; 4],
    core_layout: Layout,
    core_metrics: DockMetrics,
    one_tab_column_group: NodeId,
    two_tab_column_group: NodeId,
    one_tab_headerless_group: NodeId,
}

impl MainWindow {
    pub(crate) fn new(theme: ui::Theme) -> Result<Self, String> {
        let (
            core_layout,
            core_metrics,
            one_tab_column_group,
            two_tab_column_group,
            one_tab_headerless_group,
        ) = build_core_layout(&theme)?;

        Ok(Self {
            theme,
            dockers_menu_open: false,
            checked_panels: [false; 4],
            core_layout,
            core_metrics,
            one_tab_column_group,
            two_tab_column_group,
            one_tab_headerless_group,
        })
    }

    fn activate_tab(&mut self, group: NodeId, panel: PanelId, cx: &mut Context<Self>) {
        if let Err(error) = self.core_layout.apply(
            DockCommand::ActivateTab { group, panel },
            &self.core_metrics,
        ) {
            eprintln!("Could not activate dock tab: {error:?}");
        }
        cx.notify();
    }

    fn render_core_groups(
        &self,
        width: f32,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let viewport = Rect::new(Point::ZERO, Size::new(width.max(1.0), height.max(1.0)));
        let rects = match self.core_layout.compute_rects(viewport, &self.core_metrics) {
            Ok(rects) => rects,
            Err(error) => {
                return div()
                    .relative()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.canvas_background))
                    .text_color(rgb(self.theme.colors.muted_text))
                    .child(format!("Не удалось рассчитать компоновку Core: {error:?}"));
            }
        };

        let mut root = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.canvas_background));

        let group_specs = [
            (self.one_tab_column_group, true),
            (self.two_tab_column_group, true),
            (self.one_tab_headerless_group, false),
        ];

        for (group_id, has_header) in group_specs {
            let Some(group_rect) = rects.groups.iter().find(|entry| entry.group == group_id) else {
                continue;
            };
            let Some(tabs_view) = group_tabs_view(&self.core_layout, group_id) else {
                continue;
            };

            let first_group = group_id;
            let first_panel = tabs_view.first_panel;
            let on_first_tab = cx.listener(move |this, _, _, cx| {
                this.activate_tab(first_group, first_panel, cx);
            });

            let second_group = group_id;
            let second_panel = tabs_view.second_panel;
            let on_second_tab = cx.listener(move |this, _, _, cx| {
                if let Some(panel) = second_panel {
                    this.activate_tab(second_group, panel, cx);
                }
            });

            let group_element = if has_header {
                column_tabs_group::render_with_tabs(
                    self.theme,
                    tabs_view.active_tab,
                    group_rect.rect.size.width,
                    group_rect.rect.size.height,
                    tabs_view.first_label,
                    tabs_view.second_label,
                    on_first_tab,
                    on_second_tab,
                )
            } else {
                tabs_group::render_with_tabs(
                    self.theme,
                    tabs_view.active_tab,
                    group_rect.rect.size.width,
                    group_rect.rect.size.height,
                    tabs_view.first_label,
                    tabs_view.second_label,
                    on_first_tab,
                    on_second_tab,
                )
            };

            root = root.child(
                div()
                    .absolute()
                    .left(px(group_rect.rect.origin.x))
                    .top(px(group_rect.rect.origin.y))
                    .w(px(group_rect.rect.size.width))
                    .h(px(group_rect.rect.size.height))
                    .child(group_element),
            );
        }

        // Core provides the splitter hit zones. GPUI starts the drag while Core
        // owns all sizing rules and applies each pointer delta transactionally.
        for (splitter_index, splitter) in rects.splitters.into_iter().enumerate() {
            let available_height = match self.core_layout.node(splitter.owner) {
                Some(Node::Column(column)) => rects
                    .columns
                    .iter()
                    .find(|entry| entry.column == splitter.owner)
                    .map(|entry| {
                        (entry.rect.size.height
                            - self.core_metrics.splitter_thickness
                                * column.groups.len().saturating_sub(1) as f32)
                            .max(1.0)
                    })
                    .unwrap_or(1.0),
                _ => 1.0,
            };

            let drag_state = SplitterDragState {
                layout_before: self.core_layout.clone(),
                mouse_position_before: window.mouse_position(),
                owner: splitter.owner,
                boundary_index: splitter.boundary_index,
                axis: splitter.axis,
                available_height,
            };
            let cursor = match splitter.axis {
                dock_core::Axis::X => CursorStyle::ResizeColumn,
                dock_core::Axis::Y => CursorStyle::ResizeRow,
            };
            let (bar_left, bar_top, bar_width, bar_height) = match splitter.axis {
                dock_core::Axis::X => (
                    (splitter.rect.size.width - self.core_metrics.splitter_thickness).max(0.0) / 2.0,
                    0.0,
                    self.core_metrics.splitter_thickness.min(splitter.rect.size.width),
                    splitter.rect.size.height,
                ),
                dock_core::Axis::Y => (
                    0.0,
                    (splitter.rect.size.height - self.core_metrics.splitter_thickness).max(0.0) / 2.0,
                    splitter.rect.size.width,
                    self.core_metrics.splitter_thickness.min(splitter.rect.size.height),
                ),
            };

            let handle = div()
                .id(format!("dock-splitter-{splitter_index}"))
                .absolute()
                .left(px(splitter.rect.origin.x))
                .top(px(splitter.rect.origin.y))
                .w(px(splitter.rect.size.width))
                .h(px(splitter.rect.size.height))
                .cursor(cursor)
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_drag(drag_state, |_, _, _, cx| cx.new(|_| SplitterDragPreview))
                .on_drag_move::<SplitterDragState>(cx.listener(
                    move |this, event: &DragMoveEvent<SplitterDragState>, _window, cx| {
                        let drag = event.drag(cx);
                        let movement = event.event.position - drag.mouse_position_before;
                        let delta = match drag.axis {
                            dock_core::Axis::X => movement.x.as_f32(),
                            dock_core::Axis::Y => movement.y.as_f32(),
                        };
                        if !delta.is_finite() || delta.abs() < 0.01 {
                            return;
                        }

                        // Re-apply the absolute mouse displacement to the captured
                        // state; do not add the same cumulative delta on every frame.
                        this.core_layout = drag.layout_before.clone();
                        let command = match drag.axis {
                            dock_core::Axis::X => DockCommand::ResizeColumnBoundary {
                                side: DockSide::Right,
                                boundary_index: drag.boundary_index,
                                delta,
                            },
                            dock_core::Axis::Y => DockCommand::ResizeGroups {
                                column: drag.owner,
                                boundary_index: drag.boundary_index,
                                delta,
                                available_height: drag.available_height,
                            },
                        };
                        if let Err(error) =
                            this.core_layout.apply(command, &this.core_metrics)
                        {
                            eprintln!("Could not resize dock splitter: {error:?}");
                            this.core_layout = drag.layout_before.clone();
                        }
                        cx.notify();
                    },
                ))
                .child(
                    div()
                        .absolute()
                        .left(px(bar_left))
                        .top(px(bar_top))
                        .w(px(bar_width))
                        .h(px(bar_height))
                        .bg(rgb(self.theme.colors.splitter)),
                );

            root = root.child(handle);
        }

        root
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);
        let menu_bar_height = self.theme.metrics.column_header_height.max(1.0);
        let dropdown_width = self.theme.metrics.default_column_width.max(1.0);
        let menu_item_height = 30.0;
        let dropdown_height = if self.dockers_menu_open {
            menu_item_height * 4.0 + 2.0
        } else {
            0.0
        };

        let dropdown = if self.dockers_menu_open {
            div()
                .flex()
                .flex_col()
                .w(px(dropdown_width))
                .bg(rgb(self.theme.colors.toolbar_background))
                .border_1()
                .border_color(rgb(self.theme.colors.border))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 1",
                    self.checked_panels[0],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[0] = !this.checked_panels[0];
                        cx.notify();
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 2",
                    self.checked_panels[1],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[1] = !this.checked_panels[1];
                        cx.notify();
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 3",
                    self.checked_panels[2],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[2] = !this.checked_panels[2];
                        cx.notify();
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 4",
                    self.checked_panels[3],
                    cx.listener(|this, _, _, cx| {
                        this.checked_panels[3] = !this.checked_panels[3];
                        cx.notify();
                    }),
                ))
        } else {
            div().w(px(0.0)).h(px(0.0))
        };

        let content_height = (height - menu_bar_height - dropdown_height).max(1.0);

        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.canvas_background))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(width))
                    .h(px(menu_bar_height))
                    .bg(rgb(self.theme.colors.toolbar_background))
                    .border_1()
                    .border_color(rgb(self.theme.colors.border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(100.0))
                            .h(px(menu_bar_height))
                            .bg(rgb(self.theme.colors.toolbar_background))
                            .text_color(rgb(self.theme.colors.primary_text))
                            .border_1()
                            .border_color(rgb(self.theme.colors.border))
                            .child("Dockers ▾")
                            .id("dockers-menu-button")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dockers_menu_open = !this.dockers_menu_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(dropdown)
            .child(self.render_core_groups(width, content_height, window, cx))
    }
}

/// A menu row with a right-side checkmark that can be toggled independently.
fn render_menu_item<F>(
    theme: ui::Theme,
    width: f32,
    height: f32,
    label: &'static str,
    is_checked: bool,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .text_color(rgb(theme.colors.muted_text))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(
            div()
                .flex_1()
                .h(px(height))
                .items_center()
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(32.0))
                .h(px(height))
                .text_color(rgb(theme.colors.primary_text))
                .child(if is_checked { "✓" } else { "" }),
        )
        .id(format!("dockers-menu-item-{label}"))
        .on_click(on_click)
}

fn build_core_layout(
    theme: &ui::Theme,
) -> Result<(Layout, DockMetrics, NodeId, NodeId, NodeId), String> {
    let metrics = dock_gpui::dock_metrics_from_theme(theme);
    let constraints = PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(theme.metrics.default_column_width.max(120.0), 220.0),
        None,
        false,
    );
    let descriptors = ["layers", "color", "properties", "history"]
        .into_iter()
        .map(|key| {
            PanelKey::new(key)
                .map(|key| (key, constraints))
                .map_err(str::to_owned)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .map_err(str::to_owned)?;
    let (one_tab_column_group, two_tab_column_group) = {
        let area = layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Core factory did not create a right dock area".to_owned())?;
        let Some(Node::Column(column)) = layout.node(area.root) else {
            return Err("Core factory right dock root is not a Column".to_owned());
        };
        if column.groups.len() != 2 {
            return Err("Core factory did not create the expected two tab groups".to_owned());
        }
        (column.groups[0], column.groups[1])
    };

    let color_panel = panel_id_for_key(&layout, "color")?;
    let history_panel = panel_id_for_key(&layout, "history")?;

    layout
        .apply(
            DockCommand::MovePanel {
                panel: color_panel,
                target: DropTarget::Tabs {
                    group: two_tab_column_group,
                    index: 0,
                },
            },
            &metrics,
        )
        .map_err(|error| format!("Could not place the second tab group: {error:?}"))?;

    layout
        .apply(
            DockCommand::MovePanel {
                panel: history_panel,
                target: DropTarget::GroupEdge {
                    group: two_tab_column_group,
                    zone: DropZone::Bottom,
                },
            },
            &metrics,
        )
        .map_err(|error| format!("Could not create the lower group: {error:?}"))?;

    layout
        .apply(
            DockCommand::MoveGroupToMainColumn {
                group: one_tab_column_group,
                side: DockSide::Right,
                index: 0,
            },
            &metrics,
        )
        .map_err(|error| format!("Could not create the adjacent right column: {error:?}"))?;

    let one_tab_headerless_group = group_containing_panel(&layout, history_panel)
        .ok_or_else(|| "Core layout did not retain the lower panel group".to_owned())?;
    layout
        .validate(&metrics)
        .map_err(|errors| format!("Constructed Core layout is invalid: {errors:?}"))?;

    Ok((
        layout,
        metrics,
        one_tab_column_group,
        two_tab_column_group,
        one_tab_headerless_group,
    ))
}

fn panel_id_for_key(layout: &Layout, key: &str) -> Result<PanelId, String> {
    layout
        .panel_instances()
        .find_map(|(id, instance)| (instance.key.as_str() == key).then_some(id))
        .ok_or_else(|| format!("Core layout does not contain the panel '{key}'"))
}

fn group_containing_panel(layout: &Layout, panel_id: PanelId) -> Option<NodeId> {
    layout.nodes().find_map(|(node_id, node)| {
        let Node::Tabs(tabs) = node else {
            return None;
        };
        tabs.children.iter().any(|child_id| {
            matches!(layout.node(*child_id), Some(Node::Panel(panel)) if panel.panel_id == panel_id)
        }).then_some(node_id)
    })
}

fn group_tabs_view(layout: &Layout, group_id: NodeId) -> Option<GroupTabsView> {
    let Some(Node::Tabs(tabs)) = layout.node(group_id) else {
        return None;
    };
    let mut children = tabs.children.iter().take(2).filter_map(|child_id| {
        let Some(Node::Panel(panel)) = layout.node(*child_id) else {
            return None;
        };
        let instance = layout.panel_instance(panel.panel_id)?;
        let label = match instance.key.as_str() {
            "layers" => "Вкладка 1",
            "color" => "Вкладка 2",
            "properties" => "Вкладка 3",
            "history" => "Вкладка 4",
            _ => "Панель",
        };
        Some((panel.panel_id, label))
    });

    let (first_panel, first_label) = children.next()?;
    let second = children.next();
    Some(GroupTabsView {
        active_tab: tabs.active_index.min(if second.is_some() { 1 } else { 0 }),
        first_panel,
        first_label,
        second_panel: second.map(|(panel, _)| panel),
        second_label: second.map(|(_, label)| label),
    })
}


#[cfg(test)]
mod tests {
    use super::build_core_layout;
    use dock_core::{Axis, DockSide, Node, Point, Rect, Size};

    #[test]
    fn core_layout_places_one_tab_column_left_of_two_stacked_groups() {
        let theme = ui::Theme::default();
        let (layout, metrics, left_group, upper_group, lower_group) =
            build_core_layout(&theme).expect("initial Core layout should be valid");

        assert_eq!(layout.validate(&metrics), Ok(()));

        let right_area = layout.main_area(DockSide::Right).unwrap();
        let Node::Split(main_split) = layout.node(right_area.root).unwrap() else {
            panic!("right dock root should be the horizontal split between columns");
        };
        assert_eq!(main_split.axis, Axis::X);
        assert_eq!(main_split.children.len(), 2);

        let Node::Column(left_column) = layout.node(main_split.children[0]).unwrap() else {
            panic!("first split child should be the left column");
        };
        let Node::Column(right_column) = layout.node(main_split.children[1]).unwrap() else {
            panic!("second split child should be the right column");
        };
        assert_eq!(left_column.groups, vec![left_group]);
        assert_eq!(right_column.groups, vec![upper_group, lower_group]);

        for (group_id, expected_tabs) in [(left_group, 1), (upper_group, 2), (lower_group, 1)] {
            let Node::Tabs(tabs) = layout.node(group_id).unwrap() else {
                panic!("column child should be a Tabs group");
            };
            assert_eq!(tabs.children.len(), expected_tabs);
        }

        let rects = layout
            .compute_rects(
                Rect::new(Point::ZERO, Size::new(1200.0, 768.0)),
                &metrics,
            )
            .expect("initial tree should produce geometry");
        assert!(rects.groups.iter().any(|entry| entry.group == left_group));
        assert!(rects.groups.iter().any(|entry| entry.group == upper_group));
        assert!(rects.groups.iter().any(|entry| entry.group == lower_group));
        assert!(rects.splitters.iter().any(|splitter| splitter.axis == Axis::X));
        assert!(rects.splitters.iter().any(|splitter| splitter.axis == Axis::Y));
    }
}
