use dock_core::{ColumnMode, DockCommand, DockSide, Node, NodeId, PanelId};
use gpui::{
    Context, CursorStyle, Div, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, Render, StatefulInteractiveElement, Styled,
    Window, div, px, rgb,
};

use super::{GroupResizeState, MainWindow, TabGroupViewData};

impl MainWindow {
    fn visible_tab_groups_in_column(
        &self,
        column_id: NodeId,
    ) -> Vec<(usize, TabGroupViewData)> {
        self.group_ids_in_column(column_id)
            .into_iter()
            .enumerate()
            .filter_map(|(group_index, group_id)| {
                let mut group = self.tab_group_view_data(group_id)?;
                group
                    .panels
                    .retain(|panel_id| self.panel_is_visible(*panel_id));
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
        // A context menu needs a real Core group; avoid stale placeholder IDs.
        let header_context_group = core_group_ids.first().copied();
        let column_width = self.column_width(column_index);
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let tabs_collapsed = matches!(
            self.layout.node(column_id),
            Some(Node::Column(column)) if column.mode == ColumnMode::IconsOnly
        );

        let header_height = metrics.column_header_height.min(window_height);
        let available_below_header = (window_height - header_height).max(0.0);
        let visible_groups = self.visible_tab_groups_in_column(column_id);
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

        let column_label = self.column_label(column_index);
        let mut rendered_groups: Vec<(usize, Div, f32)> = Vec::new();
        if !tabs_collapsed {
            for ((group_index, group), group_height) in
                visible_groups.iter().zip(group_heights.iter().copied())
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
                    // Tab identity comes from its position in the Core group, not
                    // from a fixed application-owned panel slot. This also remains
                    // unique when a group contains multiple instances of one panel type.
                    let element_id = format!(
                        "dock-column-{column_index}-group-{group_index}-tab-{tab_index}"
                    );
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

                let menu_element_id =
                    format!("dock-column-{column_index}-group-{group_index}-menu");
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
                            .id(match column_index {
                                0 => "left-dock-header-spacer".to_owned(),
                                1 => "right-dock-header-spacer".to_owned(),
                                index => format!("dock-column-{index}-header-spacer"),
                            })
                            .w(px(header_spacer_width))
                            .h(px(header_height))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    if let Some(group) = header_context_group {
                                        this.open_header_context_menu(
                                            column_index,
                                            group,
                                            event.position.x.as_f32(),
                                            event.position.y.as_f32(),
                                        );
                                    }
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
                                            column_index,
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
                            .id(match column_index {
                                0 => "left-dock-collapse-tabs".to_owned(),
                                1 => "right-dock-collapse-tabs".to_owned(),
                                index => format!("dock-column-{index}-collapse-tabs"),
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
                                    if let Some(group) = header_context_group {
                                        this.open_header_context_menu(
                                            column_index,
                                            group,
                                            event.position.x.as_f32(),
                                            event.position.y.as_f32(),
                                        );
                                    }
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
                            .id(match column_index {
                                0 => "left-dock-close-column".to_owned(),
                                1 => "right-dock-close-column".to_owned(),
                                index => format!("dock-column-{index}-close-column"),
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

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let window_width = viewport.width.as_f32().max(1.0);
        let window_height = viewport.height.as_f32().max(1.0);
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;
        let menu_height = metrics.column_header_height.min(window_height);
        let dock_height = (window_height - menu_height).max(1.0);

        self.sync_column_slots_from_core();
        // The Core Split is the source of both column order and column membership.
        // Stable indices only connect current Core node IDs to per-column UI state.
        let column_indices = self.main_column_indices_in_core_order();
        let column_widths: Vec<f32> = column_indices
            .iter()
            .map(|column_index| self.column_width(*column_index).min(window_width))
            .collect();
        let any_column_open = !column_indices.is_empty();
        let inner_splitter_count = column_indices.len().saturating_sub(1);
        let splitter_width = metrics.splitter_thickness;
        let outer_splitter_width = if any_column_open { splitter_width } else { 0.0 };
        let center_splitter_width = splitter_width * inner_splitter_count as f32;
        let columns_width = column_widths.iter().sum::<f32>();
        let dock_width = columns_width + outer_splitter_width + center_splitter_width;
        let workspace_width = (window_width - dock_width).max(0.0);
        let split_hit_width = metrics.splitter_hit_zone.max(splitter_width);
        let outer_splitter_center_x = workspace_width + outer_splitter_width / 2.0;

        let outer_column_splitter = if let Some(outer_column_index) =
            column_indices.first().copied()
        {
            let outer_column_id = self.column_ids[outer_column_index];
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

        let mut dock_area = div()
            .flex()
            .flex_row()
            .w(px(window_width))
            .h(px(dock_height))
            .bg(rgb(colors.canvas_background))
            .child(div().w(px(workspace_width)).h(px(dock_height)));

        if any_column_open {
            dock_area = dock_area.child(outer_column_splitter);
            for (position, column_index) in column_indices.iter().copied().enumerate() {
                if position > 0 {
                    let boundary_index = position - 1;
                    let columns_before_boundary =
                        column_widths.iter().take(position).sum::<f32>();
                    let splitter_center_x = workspace_width
                        + outer_splitter_width
                        + columns_before_boundary
                        + splitter_width * boundary_index as f32
                        + splitter_width / 2.0;
                    let column_splitter = div()
                        .id(format!("dock-column-splitter-{boundary_index}"))
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
                                        this.resizing_column_boundary_index = boundary_index;
                                        this.resizing_outer_column = None;
                                        this.resize_column_pointer_offset_x =
                                            event.position.x.as_f32() - splitter_center_x;
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
                    dock_area = dock_area.child(column_splitter);
                }

                dock_area = dock_area.child(self.render_column(
                    cx,
                    column_index,
                    dock_height,
                    menu_height,
                ));
            }
        }

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
            for entry in &self.panel_visibility {
                let panel_id = entry.panel_id;
                let visible = entry.visible;
                let title = entry.title.clone();
                let item_id = format!(
                    "windows-panel-{}-{}",
                    entry.column_index,
                    entry.panel_key
                );
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
                        this.toggle_panel_visibility(panel_id);
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
                    let column_indices = this.main_column_indices_in_core_order();
                    let Some(outer_column_index) = column_indices.first().copied() else {
                        this.resizing_outer_column = None;
                        cx.notify();
                        return;
                    };
                    if this.column_ids.get(outer_column_index).copied() != Some(column_id) {
                        this.resizing_outer_column = None;
                        cx.notify();
                        return;
                    }

                    let splitter_width = this.theme.metrics.splitter_thickness;
                    let column_widths: Vec<f32> = column_indices
                        .iter()
                        .map(|index| this.column_width(*index).min(window_width))
                        .collect();
                    let all_splitter_width = splitter_width * column_indices.len() as f32;
                    let workspace_width = (window_width
                        - column_widths.iter().sum::<f32>()
                        - all_splitter_width)
                        .max(0.0);
                    let current_splitter_center_x = workspace_width + splitter_width / 2.0;
                    let target_splitter_center_x =
                        pointer_x - this.resize_column_pointer_offset_x;
                    let current_outer_width = column_widths.first().copied().unwrap_or(0.0);
                    let width_delta = current_splitter_center_x - target_splitter_center_x;
                    let requested_width = (current_outer_width + width_delta).max(1.0);
                    let other_columns_width = column_widths.iter().skip(1).sum::<f32>();
                    let max_width = (window_width
                        - other_columns_width
                        - all_splitter_width
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
                    let column_indices = this.main_column_indices_in_core_order();
                    let boundary_index = this.resizing_column_boundary_index;
                    if boundary_index >= column_indices.len().saturating_sub(1) {
                        this.is_resizing_column = false;
                        cx.notify();
                        return;
                    }

                    let splitter_width = this.theme.metrics.splitter_thickness;
                    let column_widths: Vec<f32> = column_indices
                        .iter()
                        .map(|index| this.column_width(*index).min(window_width))
                        .collect();
                    let all_splitter_width = splitter_width * column_indices.len() as f32;
                    let workspace_width = (window_width
                        - column_widths.iter().sum::<f32>()
                        - all_splitter_width)
                        .max(0.0);
                    let columns_before_boundary =
                        column_widths.iter().take(boundary_index + 1).sum::<f32>();
                    let current_splitter_center_x = workspace_width
                        + splitter_width
                        + columns_before_boundary
                        + splitter_width * boundary_index as f32
                        + splitter_width / 2.0;
                    let target_splitter_center_x =
                        pointer_x - this.resize_column_pointer_offset_x;
                    let delta_x = target_splitter_center_x - current_splitter_center_x;
                    if delta_x != 0.0 {
                        if let Err(error) = this.apply_dock_command(
                            DockCommand::ResizeColumnBoundary {
                                side: DockSide::Right,
                                boundary_index,
                                delta: delta_x,
                            }) {
                            eprintln!("Could not resize dock column boundary: {error}");
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
                    let visible_groups = this.visible_tab_groups_in_column(column_id);
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
