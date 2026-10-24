mod column_tabs_group;
mod floating_column_tabs_group;
mod floating_tabs_group;
mod tabs_group;

use std::collections::{HashMap, HashSet};

use dock_core::{
    create_initial_layout, DockCommand, DockMetrics, DockSide, DropTarget, DropZone,
    FloatingGroupId, Layout, Node, NodeId, PanelConstraints, PanelId, PanelKey, Point,
    Profile, Rect, Size,
};
use gpui::{
    AppContext, Context, CursorStyle, DragMoveEvent, InteractiveElement, IntoElement, MouseButton,
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

/// Meaning of a splitter gesture, including the canvas-facing outer edge.
#[derive(Clone, Copy)]
enum SplitterResizeTarget {
    ColumnBoundary {
        side: DockSide,
        boundary_index: usize,
    },
    ColumnOuterEdge {
        side: DockSide,
        column: NodeId,
        width_before: f32,
        max_width: f32,
    },
    Groups {
        column: NodeId,
        boundary_index: usize,
        available_height: f32,
    },
}

/// Snapshot captured when a splitter drag begins. Each pointer update is applied
/// to this same baseline, so the total pointer delta is not accumulated repeatedly.
#[derive(Clone)]
struct SplitterDragState {
    layout_before: Layout,
    mouse_position_before: gpui::Point<gpui::Pixels>,
    axis: dock_core::Axis,
    target: SplitterResizeTarget,
}

/// GPUI requires a view entity for a drag; resizing itself is handled by the Core layout.
struct SplitterDragPreview;

impl Render for SplitterDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Floating content initially uses half of the source group's content height.
const FLOATING_HEIGHT_RATIO: f32 = 0.5;

/// The source snapshot and geometry captured when a tab drag begins.
#[derive(Clone)]
pub(super) struct TabDragPayload {
    layout_before: Layout,
    panel: PanelId,
    group: NodeId,
    mouse_position_before: gpui::Point<gpui::Pixels>,
    drag_viewport_size: Size,
    bounds_before: Rect,
    floating_group: Option<FloatingGroupId>,
}

/// GPUI requires a view entity to start a drag. The floating component is rendered
/// from Core geometry while the pointer moves.
pub(super) struct TabDragPreview;

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Root application window.
pub(crate) struct MainWindow {
    theme: ui::Theme,
    dockers_menu_open: bool,
    core_layout: Layout,
    core_metrics: DockMetrics,
    // Header presence is UI presentation state, not a field in Core's Tabs node.
    // Preserve it by group identity when a tab becomes a new floating group.
    group_header_modes: HashMap<NodeId, bool>,
}

impl MainWindow {
    pub(crate) fn new(theme: ui::Theme) -> Result<Self, String> {
        let (
            core_layout,
            core_metrics,
            one_tab_headered_group,
            two_tab_headered_group,
            one_tab_headerless_group,
        ) = build_core_layout(&theme)?;
        let group_header_modes = HashMap::from([
            (one_tab_headered_group, true),
            (two_tab_headered_group, true),
            (one_tab_headerless_group, false),
        ]);

        Ok(Self {
            theme,
            dockers_menu_open: false,
            core_layout,
            core_metrics,
            group_header_modes,
        })
    }

    fn has_group_header(&self, group_id: NodeId) -> bool {
        self.group_header_modes
            .get(&group_id)
            .copied()
            .unwrap_or_else(|| group_has_header(&self.core_layout, group_id))
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

    fn update_splitter_from_drag(
        &mut self,
        event: &DragMoveEvent<SplitterDragState>,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        let movement = event.event.position - drag.mouse_position_before;
        let delta = match drag.axis {
            dock_core::Axis::X => movement.x.as_f32(),
            dock_core::Axis::Y => movement.y.as_f32(),
        };
        if !delta.is_finite() || delta.abs() < 0.01 {
            return;
        }

        // Re-apply the absolute mouse displacement to the captured state; do not
        // accumulate the same displacement repeatedly across move events.
        self.core_layout = drag.layout_before.clone();
        let command = match drag.target {
            SplitterResizeTarget::ColumnBoundary {
                side,
                boundary_index,
            } => DockCommand::ResizeColumnBoundary {
                side,
                boundary_index,
                delta,
            },
            SplitterResizeTarget::ColumnOuterEdge {
                side,
                column,
                width_before,
                max_width,
            } => {
                // The canvas-facing edge is the right edge for a left dock and
                // the left edge for a right dock.
                let width_delta = match side {
                    DockSide::Left => movement.x.as_f32(),
                    DockSide::Right => -movement.x.as_f32(),
                };
                DockCommand::ResizeColumn {
                    column,
                    width: (width_before + width_delta).min(max_width),
                }
            }
            SplitterResizeTarget::Groups {
                column,
                boundary_index,
                available_height,
            } => DockCommand::ResizeGroups {
                column,
                boundary_index,
                delta,
                available_height,
            },
        };

        if let Err(error) = self.core_layout.apply(command, &self.core_metrics) {
            eprintln!("Could not resize dock splitter: {error:?}");
            self.core_layout = drag.layout_before.clone();
        }
        cx.notify();
    }

    fn toggle_panel(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let constraints = panel_constraints(&self.theme);
        if let Err(error) = toggle_core_panel(
            &mut self.core_layout,
            &self.core_metrics,
            constraints,
            key,
        ) {
            eprintln!("Could not toggle dock panel '{key}': {error}");
        }
        synchronize_group_header_modes(&self.core_layout, &mut self.group_header_modes);
        cx.notify();
    }

    fn tab_drag_payload(
        &self,
        panel: PanelId,
        group: NodeId,
        group_rect: Rect,
        mouse_position_before: gpui::Point<gpui::Pixels>,
        drag_viewport_size: Size,
    ) -> TabDragPayload {
        let floating = floating_group_for_panel(&self.core_layout, panel);
        let (floating_group, bounds_before) = match floating {
            Some((floating_group, bounds)) => (Some(floating_group), bounds),
            None => (None, group_rect),
        };

        TabDragPayload {
            layout_before: self.core_layout.clone(),
            panel,
            group,
            mouse_position_before,
            drag_viewport_size,
            bounds_before,
            floating_group,
        }
    }

    fn update_tab_from_drag(
        &mut self,
        event: &DragMoveEvent<TabDragPayload>,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        let source_has_header = self
            .group_header_modes
            .get(&drag.group)
            .copied()
            .unwrap_or_else(|| group_has_header(&drag.layout_before, drag.group));
        let source_tab_count = match drag.layout_before.node(drag.group) {
            Some(Node::Tabs(tabs)) => tabs.children.len(),
            _ => 0,
        };
        let current_pointer = Point::new(
            event.event.position.x.as_f32(),
            event.event.position.y.as_f32(),
        );
        let pointer_before = Point::new(
            drag.mouse_position_before.x.as_f32(),
            drag.mouse_position_before.y.as_f32(),
        );
        let movement =
            clamped_pointer_delta(current_pointer, pointer_before, drag.drag_viewport_size);
        let dx = movement.x;
        let dy = movement.y;
        if !dx.is_finite() || !dy.is_finite() || (dx.abs() < 0.01 && dy.abs() < 0.01) {
            return;
        }

        // Move the frame by the pointer delta, not by clamping its outer edge.
        // This keeps the grabbed tab point under the cursor while the root clips
        // any part of the floating frame that passes the app-window boundary.
        let origin = Point::new(
            drag.bounds_before.origin.x + dx,
            drag.bounds_before.origin.y + dy,
        );
        let floating_size = if drag.floating_group.is_some() {
            // Once floating, keep the actual size stable while moving it.
            drag.bounds_before.size
        } else {
            // Keep the title/tab chrome at full height and shrink only the content
            // area. The headerless History group uses the matching no-header sizing.
            floating_size_for_detach(
                drag.bounds_before,
                source_has_header,
                &self.core_metrics,
            )
        };
        let bounds = Rect::new(origin, floating_size);

        // Rebuild from the drag-start snapshot so cumulative mouse movement is
        // applied once and never added repeatedly to the previously moved frame.
        self.core_layout = drag.layout_before.clone();
        let command = if let Some(floating_group) = drag.floating_group {
            DockCommand::MoveFloating {
                group: floating_group,
                origin,
            }
        } else {
            if source_tab_count > 1 {
                DockCommand::FloatPanelAsGroup {
                    panel: drag.panel,
                    bounds,
                }
            } else if let Some((column, group_count)) =
                column_containing_group(&self.core_layout, drag.group)
            {
                if group_count > 1 {
                    DockCommand::FloatGroupAsColumn {
                        group: drag.group,
                        bounds,
                    }
                } else {
                    // The dragged tab is the whole column. Float that containing
                    // composition instead of leaving an invalid empty Column behind.
                    DockCommand::Float {
                        node: column,
                        bounds,
                    }
                }
            } else {
                // Generic fallback for a tab group that is not owned by a Column.
                DockCommand::Float {
                    node: drag.group,
                    bounds,
                }
            }
        };

        let applied = match self.core_layout.apply(command, &self.core_metrics) {
            Ok(_) => true,
            Err(error) => {
                eprintln!("Could not float dragged tab: {error:?}");
                self.core_layout = drag.layout_before.clone();
                false
            }
        };

        synchronize_group_header_modes(&self.core_layout, &mut self.group_header_modes);
        if applied && drag.floating_group.is_none() && source_tab_count > 1 {
            // FloatPanelAsGroup creates a new Tabs node. Its presentation mode comes
            // from the source group, not from the detached panel's key/name.
            inherit_group_header_mode_for_panel(
                &self.core_layout,
                &mut self.group_header_modes,
                drag.panel,
                source_has_header,
            );
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
        let window_viewport = window.viewport_size();
        let drag_viewport_size = Size::new(
            window_viewport.width.as_f32().max(1.0),
            window_viewport.height.as_f32().max(1.0),
        );
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
            .overflow_hidden()
            .bg(rgb(self.theme.colors.canvas_background));

        for group_rect in &rects.groups {
            let group_id = group_rect.group;
            if group_is_floating(&self.core_layout, group_id) {
                continue;
            }
            let has_header = self.has_group_header(group_id);
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

            let drag_mouse_position = window.mouse_position();
            let first_drag = Some(self.tab_drag_payload(
                tabs_view.first_panel,
                group_id,
                group_rect.rect,
                drag_mouse_position,
                drag_viewport_size,
            ));
            let second_drag = tabs_view.second_panel.map(|panel| {
                self.tab_drag_payload(
                    panel,
                    group_id,
                    group_rect.rect,
                    drag_mouse_position,
                    drag_viewport_size,
                )
            });

            let group_element = if has_header {
                column_tabs_group::render(
                    self.theme,
                    tabs_view.active_tab,
                    group_rect.rect.size.width,
                    group_rect.rect.size.height,
                    tabs_view.first_label,
                    tabs_view.second_label,
                    first_drag,
                    second_drag,
                    on_first_tab,
                    on_second_tab,
                )
            } else {
                tabs_group::render(
                    self.theme,
                    tabs_view.active_tab,
                    group_rect.rect.size.width,
                    group_rect.rect.size.height,
                    tabs_view.first_label,
                    tabs_view.second_label,
                    first_drag,
                    second_drag,
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

            let target = match splitter.axis {
                dock_core::Axis::X => SplitterResizeTarget::ColumnBoundary {
                    side: DockSide::Right,
                    boundary_index: splitter.boundary_index,
                },
                dock_core::Axis::Y => SplitterResizeTarget::Groups {
                    column: splitter.owner,
                    boundary_index: splitter.boundary_index,
                    available_height,
                },
            };
            let drag_state = SplitterDragState {
                layout_before: self.core_layout.clone(),
                mouse_position_before: window.mouse_position(),
                axis: splitter.axis,
                target,
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
                        this.update_splitter_from_drag(event, cx);
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

        // The left edge of the right dock is its canvas-facing outer boundary.
        // Core emits splitters only between siblings, so this edge uses ResizeColumn.
        let right_dock_outer_column = self.core_layout.main_area(DockSide::Right).and_then(|area| {
            match self.core_layout.node(area.root) {
                Some(Node::Column(_)) => Some(area.root),
                Some(Node::Split(split)) if split.axis == dock_core::Axis::X => {
                    split.children.first().copied()
                }
                _ => None,
            }
        });
        if let Some(column_id) = right_dock_outer_column {
            let width_before = match self.core_layout.node(column_id) {
                Some(Node::Column(column))
                    if matches!(column.mode, dock_core::ColumnMode::Expanded) =>
                {
                    Some(column.width)
                }
                _ => None,
            };
            let column_rect = rects.columns.iter().find(|entry| entry.column == column_id);
            let canvas_rect = rects.canvas;
            if let (Some(width_before), Some(column_rect), Some(canvas_rect)) =
                (width_before, column_rect, canvas_rect)
            {
                let hit_width = self.core_metrics.splitter_hit_zone.min(width);
                let hit_height = column_rect.rect.size.height;
                if hit_width > 0.0 && hit_height > 0.0 {
                    // Do not let this outer-edge resize consume the canvas below
                    // Core's minimum canvas width, even before canvas bounds have
                    // been persisted in CanvasState.
                    let max_width = width_before
                        + (canvas_rect.size.width - self.core_metrics.min_canvas_width).max(0.0);
                    let handle_left = (column_rect.rect.origin.x - hit_width / 2.0)
                        .clamp(0.0, (width - hit_width).max(0.0));
                    let bar_width = self.core_metrics.splitter_thickness.min(hit_width);
                    let drag_state = SplitterDragState {
                        layout_before: self.core_layout.clone(),
                        mouse_position_before: window.mouse_position(),
                        axis: dock_core::Axis::X,
                        target: SplitterResizeTarget::ColumnOuterEdge {
                            side: DockSide::Right,
                            column: column_id,
                            width_before,
                            max_width,
                        },
                    };

                    let handle = div()
                        .id("dock-right-outer-left-resize")
                        .absolute()
                        .left(px(handle_left))
                        .top(px(column_rect.rect.origin.y))
                        .w(px(hit_width))
                        .h(px(hit_height))
                        .cursor(CursorStyle::ResizeColumn)
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_drag(drag_state, |_, _, _, cx| cx.new(|_| SplitterDragPreview))
                        .on_drag_move::<SplitterDragState>(cx.listener(
                            move |this, event: &DragMoveEvent<SplitterDragState>, _window, cx| {
                                this.update_splitter_from_drag(event, cx);
                            },
                        ))
                        .child(
                            div()
                                .absolute()
                                .left(px((hit_width - bar_width) / 2.0))
                                .top(px(0.0))
                                .w(px(bar_width))
                                .h(px(hit_height))
                                .bg(rgb(self.theme.colors.splitter)),
                        );

                    root = root.child(handle);
                }
            }
        }

        // Floating trees are rendered separately so they use their own component and frame.
        // Core returns their geometry in back-to-front z-order.
        for floating_rect in &rects.floating_groups {
            let Some(group_id) =
                floating_root_tabs_group(&self.core_layout, floating_rect.group)
            else {
                continue;
            };
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

            let drag_mouse_position = window.mouse_position();
            let first_drag = Some(self.tab_drag_payload(
                tabs_view.first_panel,
                group_id,
                group_rect.rect,
                drag_mouse_position,
                drag_viewport_size,
            ));
            let second_drag = tabs_view.second_panel.map(|panel| {
                self.tab_drag_payload(
                    panel,
                    group_id,
                    group_rect.rect,
                    drag_mouse_position,
                    drag_viewport_size,
                )
            });

            let floating_bounds =
                floating_group_bounds(&self.core_layout, floating_rect.group)
                    .unwrap_or(floating_rect.rect);
            // Keep the model's raw origin while dragging. Core's geometry snapshot
            // clamps the visible rect for layout calculations; using that clamped
            // origin for painting made the panel stop before the pointer reached
            // the app-window edge.
            let floating_element = if self.has_group_header(group_id) {
                floating_column_tabs_group::render(
                    self.theme,
                    tabs_view.active_tab,
                    floating_bounds.size.width,
                    floating_bounds.size.height,
                    tabs_view.first_label,
                    tabs_view.second_label,
                    first_drag,
                    second_drag,
                    on_first_tab,
                    on_second_tab,
                )
            } else {
                floating_tabs_group::render(
                    self.theme,
                    tabs_view.active_tab,
                    floating_bounds.size.width,
                    floating_bounds.size.height,
                    tabs_view.first_label,
                    tabs_view.second_label,
                    first_drag,
                    second_drag,
                    on_first_tab,
                    on_second_tab,
                )
            };

            root = root.child(
                div()
                    .absolute()
                    .left(px(floating_bounds.origin.x))
                    .top(px(floating_bounds.origin.y))
                    .w(px(floating_bounds.size.width))
                    .h(px(floating_bounds.size.height))
                    .child(floating_element),
            );
        }

        root.on_drag_move::<TabDragPayload>(cx.listener(
            |this, event: &DragMoveEvent<TabDragPayload>, _window, cx| {
                this.update_tab_from_drag(event, cx);
            },
        ))
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
                    panel_is_open(&self.core_layout, "layers"),
                    cx.listener(|this, _, _, cx| {
                        this.toggle_panel("layers", cx);
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 2",
                    panel_is_open(&self.core_layout, "color"),
                    cx.listener(|this, _, _, cx| {
                        this.toggle_panel("color", cx);
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 3",
                    panel_is_open(&self.core_layout, "properties"),
                    cx.listener(|this, _, _, cx| {
                        this.toggle_panel("properties", cx);
                    }),
                ))
                .child(render_menu_item(
                    self.theme,
                    dropdown_width,
                    menu_item_height,
                    "Вкладка 4",
                    panel_is_open(&self.core_layout, "history"),
                    cx.listener(|this, _, _, cx| {
                        this.toggle_panel("history", cx);
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
    let constraints = panel_constraints(theme);
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

fn panel_constraints(theme: &ui::Theme) -> PanelConstraints {
    PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(theme.metrics.default_column_width.max(120.0), 220.0),
        None,
        false,
    )
}

fn panel_id_if_open(layout: &Layout, key: &str) -> Option<PanelId> {
    layout
        .panel_instances()
        .find_map(|(id, instance)| (instance.key.as_str() == key).then_some(id))
}

fn panel_is_open(layout: &Layout, key: &str) -> bool {
    panel_id_if_open(layout, key).is_some()
}

fn toggle_core_panel(
    layout: &mut Layout,
    metrics: &DockMetrics,
    constraints: PanelConstraints,
    key: &str,
) -> Result<(), String> {
    if let Some(panel) = panel_id_if_open(layout, key) {
        layout
            .apply(DockCommand::ClosePanel { panel }, metrics)
            .map_err(|error| format!("could not close panel '{key}': {error:?}"))?;
        return Ok(());
    }

    let panel_key = PanelKey::new(key).map_err(str::to_owned)?;
    let target = panel_open_target(layout, key)?;
    layout
        .apply(
            DockCommand::OpenPanel {
                key: panel_key,
                constraints,
                target: Some(target),
            },
            metrics,
        )
        .map_err(|error| format!("could not open panel '{key}': {error:?}"))?;
    Ok(())
}

fn panel_open_target(layout: &Layout, key: &str) -> Result<DropTarget, String> {
    let right_column_count = main_column_count(layout, DockSide::Right);

    match key {
        // Layers always gets its own first column, to the left of the other panels.
        "layers" => Ok(DropTarget::MainColumn {
            side: DockSide::Right,
            index: 0,
        }),
        // Color and Properties belong to the same tab group whenever both are open.
        "color" => {
            if let Some(group) = group_containing_key(layout, "properties") {
                return Ok(DropTarget::Tabs { group, index: 0 });
            }
            if let Some(group) = group_containing_key(layout, "history") {
                return Ok(DropTarget::GroupEdge {
                    group,
                    zone: DropZone::Top,
                });
            }
            Ok(DropTarget::MainColumn {
                side: DockSide::Right,
                index: right_column_count,
            })
        }
        "properties" => {
            if let Some(group) = group_containing_key(layout, "color") {
                let index = match layout.node(group) {
                    Some(Node::Tabs(tabs)) => tabs.children.len(),
                    _ => {
                        return Err("the Color panel's group is no longer available".to_owned());
                    }
                };
                return Ok(DropTarget::Tabs { group, index });
            }
            if let Some(group) = group_containing_key(layout, "history") {
                return Ok(DropTarget::GroupEdge {
                    group,
                    zone: DropZone::Top,
                });
            }
            Ok(DropTarget::MainColumn {
                side: DockSide::Right,
                index: right_column_count,
            })
        }
        // History remains below the Color/Properties group whenever that group exists.
        "history" => {
            if let Some(group) = group_containing_key(layout, "color")
                .or_else(|| group_containing_key(layout, "properties"))
            {
                return Ok(DropTarget::GroupEdge {
                    group,
                    zone: DropZone::Bottom,
                });
            }
            Ok(DropTarget::MainColumn {
                side: DockSide::Right,
                index: right_column_count,
            })
        }
        _ => Err(format!("unknown dock panel key '{key}'")),
    }
}

fn main_column_count(layout: &Layout, side: DockSide) -> usize {
    let Some(area) = layout.main_area(side) else {
        return 0;
    };
    match layout.node(area.root) {
        Some(Node::Column(_)) => 1,
        Some(Node::Split(split)) if split.axis == dock_core::Axis::X => split.children.len(),
        _ => 0,
    }
}

fn group_containing_key(layout: &Layout, key: &str) -> Option<NodeId> {
    let panel = panel_id_if_open(layout, key)?;
    group_containing_panel(layout, panel)
}

fn synchronize_group_header_modes(layout: &Layout, modes: &mut HashMap<NodeId, bool>) {
    let live_tabs: HashSet<NodeId> = layout
        .nodes()
        .filter_map(|(node_id, node)| matches!(node, Node::Tabs(_)).then_some(node_id))
        .collect();
    modes.retain(|group_id, _| live_tabs.contains(group_id));
}

fn inherit_group_header_mode_for_panel(
    layout: &Layout,
    modes: &mut HashMap<NodeId, bool>,
    panel_id: PanelId,
    has_header: bool,
) {
    synchronize_group_header_modes(layout, modes);
    if let Some(group_id) = group_containing_panel(layout, panel_id) {
        modes.insert(group_id, has_header);
    }
}

// Fallback policy for groups without recorded presentation metadata.
// Initial groups and groups created by detaching a tab use explicit modes above.
fn group_has_header(layout: &Layout, group_id: NodeId) -> bool {
    let Some(Node::Tabs(tabs)) = layout.node(group_id) else {
        return false;
    };
    tabs.children.iter().any(|child_id| match layout.node(*child_id) {
        Some(Node::Panel(panel)) => layout
            .panel_instance(panel.panel_id)
            .map(|instance| instance.key.as_str() != "history")
            .unwrap_or(true),
        _ => true,
    })
}

fn clamped_pointer_delta(current: Point, start: Point, viewport_size: Size) -> Point {
    let max_x = viewport_size.width.max(1.0);
    let max_y = viewport_size.height.max(1.0);
    let current_x = current.x.clamp(0.0, max_x);
    let current_y = current.y.clamp(0.0, max_y);
    let start_x = start.x.clamp(0.0, max_x);
    let start_y = start.y.clamp(0.0, max_y);

    Point::new(current_x - start_x, current_y - start_y)
}

fn floating_size_for_detach(
    source_bounds: Rect,
    has_header: bool,
    metrics: &DockMetrics,
) -> Size {
    let header_height = if has_header {
        metrics.header_height.max(1.0)
    } else {
        0.0
    };
    let tab_strip_height = metrics.tab_strip_height.max(1.0);
    let minimum_content_height = metrics.min_group_height.max(1.0);
    let source_content_height = (source_bounds.size.height - header_height - tab_strip_height)
        .max(minimum_content_height);
    let floating_content_height =
        (source_content_height * FLOATING_HEIGHT_RATIO).max(minimum_content_height);

    // Core's minimum floating size includes its full header metric, even for the
    // app's headerless History group; preserve that lower bound for valid geometry.
    let minimum_floating_height = metrics.header_height.max(1.0)
        + metrics.tab_strip_height.max(1.0)
        + minimum_content_height;
    let height = (header_height + tab_strip_height + floating_content_height)
        .max(minimum_floating_height);

    Size::new(source_bounds.size.width, height)
}

fn floating_group_bounds(layout: &Layout, floating_id: FloatingGroupId) -> Option<Rect> {
    layout
        .floating_groups()
        .find_map(|(id, floating)| (id == floating_id).then_some(floating.bounds))
}

fn floating_group_for_panel(
    layout: &Layout,
    panel_id: PanelId,
) -> Option<(FloatingGroupId, Rect)> {
    layout.floating_groups().find_map(|(floating_id, floating)| {
        let root = layout.dock_area(floating.area)?.root;
        subtree_contains_panel(layout, root, panel_id).then_some((floating_id, floating.bounds))
    })
}

fn group_is_floating(layout: &Layout, group_id: NodeId) -> bool {
    layout.floating_groups().any(|(_, floating)| {
        layout
            .dock_area(floating.area)
            .is_some_and(|area| subtree_contains_node(layout, area.root, group_id))
    })
}

fn floating_root_tabs_group(layout: &Layout, floating_id: FloatingGroupId) -> Option<NodeId> {
    let floating = layout
        .floating_groups()
        .find_map(|(id, floating)| (id == floating_id).then_some(floating))?;
    let root = layout.dock_area(floating.area)?.root;
    first_tabs_group_in_subtree(layout, root)
}

fn first_tabs_group_in_subtree(layout: &Layout, node_id: NodeId) -> Option<NodeId> {
    match layout.node(node_id)? {
        Node::Tabs(_) => Some(node_id),
        Node::Column(column) => column
            .groups
            .iter()
            .find_map(|child| first_tabs_group_in_subtree(layout, *child)),
        Node::Split(split) => split
            .children
            .iter()
            .find_map(|child| first_tabs_group_in_subtree(layout, *child)),
        Node::Panel(_) => None,
    }
}

fn subtree_contains_node(layout: &Layout, root: NodeId, target: NodeId) -> bool {
    if root == target {
        return true;
    }
    match layout.node(root) {
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .any(|child| subtree_contains_node(layout, *child, target)),
        Some(Node::Split(split)) => split
            .children
            .iter()
            .any(|child| subtree_contains_node(layout, *child, target)),
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .any(|child| subtree_contains_node(layout, *child, target)),
        Some(Node::Panel(_)) | None => false,
    }
}

fn subtree_contains_panel(layout: &Layout, root: NodeId, target: PanelId) -> bool {
    match layout.node(root) {
        Some(Node::Panel(panel)) => panel.panel_id == target,
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .any(|child| subtree_contains_panel(layout, *child, target)),
        Some(Node::Split(split)) => split
            .children
            .iter()
            .any(|child| subtree_contains_panel(layout, *child, target)),
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .any(|child| subtree_contains_panel(layout, *child, target)),
        None => false,
    }
}

fn column_containing_group(layout: &Layout, group: NodeId) -> Option<(NodeId, usize)> {
    layout.nodes().find_map(|(node_id, node)| match node {
        Node::Column(column) if column.groups.contains(&group) => {
            Some((node_id, column.groups.len()))
        }
        _ => None,
    })
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
    use super::{
        build_core_layout, column_containing_group, floating_group_bounds,
        floating_group_for_panel, floating_root_tabs_group, group_containing_panel,
        group_has_header, group_is_floating, panel_constraints, panel_id_for_key,
        clamped_pointer_delta, panel_is_open, toggle_core_panel, floating_size_for_detach,
        synchronize_group_header_modes, inherit_group_header_mode_for_panel, FLOATING_HEIGHT_RATIO,
    };
    use dock_core::{Axis, DockCommand, DockSide, Node, Point, Rect, Size};

    #[test]
    fn floating_rendering_uses_the_correct_header_mode() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, _, upper_group, lower_group) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        assert!(group_has_header(&layout, upper_group));
        assert!(!group_has_header(&layout, lower_group));

        let color = panel_id_for_key(&layout, "color").unwrap();
        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        let rect = rects.groups.iter().find(|item| item.group == upper_group).unwrap().rect;
        let floating_size =
            floating_size_for_detach(rect, group_has_header(&layout, upper_group), &metrics);
        let bounds = Rect::new(
            Point::new(rect.origin.x + 20.0, rect.origin.y + 16.0),
            floating_size,
        );
        layout
            .apply(DockCommand::FloatPanelAsGroup { panel: color, bounds }, &metrics)
            .expect("a tab should detach into a floating group");
        let floating = layout.floating_groups().next().unwrap();
        assert!(floating_group_bounds(&layout, floating.0).is_some());
    }

    #[test]
    fn detached_panel_inherits_header_mode_from_its_source_group() {
        use std::collections::HashMap;

        let theme = ui::Theme::default();
        let (mut layout, metrics, _, headered_group, headerless_group) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let mut header_modes =
            HashMap::from([(headered_group, true), (headerless_group, false)]);
        let history = panel_id_for_key(&layout, "history").unwrap();

        // Move History into a group that explicitly has a header. Its panel key must
        // not override the source group's component type when it is later detached.
        layout
            .apply(
                DockCommand::MovePanel {
                    panel: history,
                    target: dock_core::DropTarget::Tabs {
                        group: headered_group,
                        index: 2,
                    },
                },
                &metrics,
            )
            .expect("History should join the headered group");

        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        let source_bounds = rects
            .groups
            .iter()
            .find(|entry| entry.group == headered_group)
            .expect("headered source group should have geometry")
            .rect;
        let size = floating_size_for_detach(source_bounds, true, &metrics);
        let bounds = Rect::new(
            Point::new(source_bounds.origin.x + 20.0, source_bounds.origin.y + 16.0),
            size,
        );

        layout
            .apply(
                DockCommand::FloatPanelAsGroup { panel: history, bounds },
                &metrics,
            )
            .expect("History should detach as its own floating group");
        assert_eq!(layout.validate(&metrics), Ok(()));

        let floating_tabs = group_containing_panel(&layout, history)
            .expect("detached History should belong to a new Tabs node");
        assert!(
            !group_has_header(&layout, floating_tabs),
            "the fallback based on panel key would misclassify History as headerless"
        );

        inherit_group_header_mode_for_panel(&layout, &mut header_modes, history, true);
        assert_eq!(header_modes.get(&floating_tabs), Some(&true));
    }

    #[test]
    fn floating_drag_uses_pointer_until_it_reaches_the_window_edge() {
        let viewport = Size::new(1200.0, 800.0);
        let start = Point::new(300.0, 200.0);

        let beyond_right_bottom =
            clamped_pointer_delta(Point::new(1300.0, 900.0), start, viewport);
        assert_eq!(beyond_right_bottom, Point::new(900.0, 600.0));

        let beyond_left_top =
            clamped_pointer_delta(Point::new(-20.0, -10.0), start, viewport);
        assert_eq!(beyond_left_top, Point::new(-300.0, -200.0));
    }

    #[test]
    fn floating_detach_halves_content_without_shrinking_header_or_tab_strip() {
        let theme = ui::Theme::default();
        let (_, metrics, _, _, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let source_bounds = Rect::new(Point::ZERO, Size::new(320.0, 1000.0));
        let header_height = metrics.header_height.max(1.0);
        let tab_strip_height = metrics.tab_strip_height.max(1.0);
        let minimum_content_height = metrics.min_group_height.max(1.0);
        let minimum_floating_height =
            header_height + tab_strip_height + minimum_content_height;

        let with_header = floating_size_for_detach(source_bounds, true, &metrics);
        let source_content_with_header = (source_bounds.size.height
            - header_height
            - tab_strip_height)
            .max(minimum_content_height);
        let expected_with_header = (header_height
            + tab_strip_height
            + (source_content_with_header * FLOATING_HEIGHT_RATIO).max(minimum_content_height))
            .max(minimum_floating_height);
        assert!((with_header.height - expected_with_header).abs() < 0.001);

        let without_header = floating_size_for_detach(source_bounds, false, &metrics);
        let source_content_without_header =
            (source_bounds.size.height - tab_strip_height).max(minimum_content_height);
        let expected_without_header = (tab_strip_height
            + (source_content_without_header * FLOATING_HEIGHT_RATIO).max(minimum_content_height))
            .max(minimum_floating_height);
        assert!((without_header.height - expected_without_header).abs() < 0.001);
    }

    #[test]
    fn floating_tree_helpers_find_panels_after_detach() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, left_group, upper_group, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let panel = panel_id_for_key(&layout, "color").unwrap();
        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let source_rects = layout.compute_rects(viewport, &metrics).unwrap();
        let source_group_rect = source_rects
            .groups
            .iter()
            .find(|item| item.group == upper_group)
            .unwrap()
            .rect;
        let bounds = Rect::new(
            Point::new(source_group_rect.origin.x + 24.0, source_group_rect.origin.y + 18.0),
            source_group_rect.size,
        );

        layout
            .apply(DockCommand::FloatPanelAsGroup { panel, bounds }, &metrics)
            .expect("detaching one tab from a multi-tab group should float it");
        assert_eq!(layout.validate(&metrics), Ok(()));
        let (floating_id, _) = floating_group_for_panel(&layout, panel)
            .expect("detached panel should now belong to a floating group");
        assert!(floating_root_tabs_group(&layout, floating_id).is_some());
        assert!(group_is_floating(
            &layout,
            group_containing_panel(&layout, panel).unwrap()
        ));
        layout
            .compute_rects(viewport, &metrics)
            .expect("floating geometry should be valid");
        let _ = left_group;
    }

    #[test]
    fn floating_single_tab_group_detaches_its_containing_column() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, left_group, _, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let panel = panel_id_for_key(&layout, "layers").unwrap();
        let (column, group_count) = column_containing_group(&layout, left_group).unwrap();
        assert_eq!(group_count, 1);

        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let source_rects = layout.compute_rects(viewport, &metrics).unwrap();
        let source_group_rect = source_rects
            .groups
            .iter()
            .find(|item| item.group == left_group)
            .unwrap()
            .rect;
        let bounds = Rect::new(
            Point::new(source_group_rect.origin.x + 30.0, source_group_rect.origin.y + 20.0),
            source_group_rect.size,
        );

        layout
            .apply(DockCommand::Float { node: column, bounds }, &metrics)
            .expect("floating the only group should detach its containing column");
        assert_eq!(layout.validate(&metrics), Ok(()));
        let (floating_id, _) = floating_group_for_panel(&layout, panel)
            .expect("the panel should belong to the new floating column");
        assert!(floating_root_tabs_group(&layout, floating_id).is_some());
        layout
            .compute_rects(viewport, &metrics)
            .expect("single-tab floating geometry should be valid");
    }

    #[test]
    fn menu_toggles_all_panels_off_and_back_on_without_stale_group_ids() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, _, _, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let constraints = panel_constraints(&theme);
        let keys = ["layers", "color", "properties", "history"];

        for key in keys {
            assert!(panel_is_open(&layout, key));
            toggle_core_panel(&mut layout, &metrics, constraints, key)
                .expect("menu should close an open panel");
            assert!(!panel_is_open(&layout, key));
            assert_eq!(layout.validate(&metrics), Ok(()));
        }

        for key in ["history", "properties", "color", "layers"] {
            assert!(!panel_is_open(&layout, key));
            toggle_core_panel(&mut layout, &metrics, constraints, key)
                .expect("menu should reopen a closed panel");
            assert!(panel_is_open(&layout, key));
            assert_eq!(layout.validate(&metrics), Ok(()));
            layout
                .compute_rects(
                    Rect::new(Point::ZERO, Size::new(1200.0, 768.0)),
                    &metrics,
                )
                .expect("layout geometry should remain valid after toggling");
        }
    }

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
