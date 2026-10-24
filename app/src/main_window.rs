mod tabs_group;

use dock_core::{
    create_initial_layout, DockCommand, DockMetrics, DockSide, DropTarget, DropZone,
    FloatingGroupId, Layout, Node, NodeId, PanelConstraints, PanelId, PanelKey, Point,
    Profile, Rect, Size,
};
use gpui::{
    AppContext, Context, CursorStyle, DragMoveEvent, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use std::{cell::Cell, rc::Rc};

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
    /// Updated by tab.rs at the actual drag initiation, not when the view last rendered.
    drag_start_position: Rc<Cell<gpui::Point<gpui::Pixels>>>,
    drag_viewport_size: Size,
    bounds_before: Rect,
    floating_group: Option<FloatingGroupId>,
    label: &'static str,
}

/// The full source snapshot and geometry captured when a group drag begins.
#[derive(Clone)]
pub(super) struct GroupDragPayload {
    layout_before: Layout,
    /// The dragged Tabs group, or a complete main-area root when whole_area is true.
    group: NodeId,
    drag_start_position: Rc<Cell<gpui::Point<gpui::Pixels>>>,
    drag_viewport_size: Size,
    bounds_before: Rect,
    floating_group: Option<FloatingGroupId>,
    whole_area: bool,
}

/// Temporary presentation state only. Core's Layout is not changed until drop.
#[derive(Clone)]
struct TabDragPreviewState {
    layout: Layout,
    group: NodeId,
    label: &'static str,
    bounds: Rect,
}

/// Presentation-only snapshot for moving a complete group.
#[derive(Clone)]
struct GroupDragPreviewState {
    layout: Layout,
    group: NodeId,
    bounds: Rect,
    source_bounds: Rect,
    whole_area: bool,
}

/// GPUI uses the same empty drag entity for tab and group gestures.
pub(super) struct DockDragPreview;

impl Render for DockDragPreview {
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
    tab_drag_preview: Option<TabDragPreviewState>,
    active_tab_drag_payload: Option<TabDragPayload>,
    group_drag_preview: Option<GroupDragPreviewState>,
    active_group_drag_payload: Option<GroupDragPayload>,
}

impl MainWindow {
    pub(crate) fn new(theme: ui::Theme) -> Result<Self, String> {
        let (core_layout, core_metrics, _, _, _) = build_core_layout(&theme)?;

        Ok(Self {
            theme,
            dockers_menu_open: false,
            core_layout,
            core_metrics,
            tab_drag_preview: None,
            active_tab_drag_payload: None,
            group_drag_preview: None,
            active_group_drag_payload: None,
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
        let (floating_group, group_bounds) = match floating {
            Some((floating_group, bounds)) => (Some(floating_group), bounds),
            None => (None, group_rect),
        };
        // The preview renders the dragged panel as its first and only tab.
        // Preserve the source tab's horizontal slot under the pointer when the
        // second tab is detached by shifting the preview/window origin by one slot.
        let bounds_before = tab_drag_bounds_for_panel(
            &self.core_layout,
            group,
            panel,
            group_bounds,
            self.theme.metrics.column_tab_width,
        );

        TabDragPayload {
            layout_before: self.core_layout.clone(),
            panel,
            group,
            drag_start_position: Rc::new(Cell::new(mouse_position_before)),
            drag_viewport_size,
            bounds_before,
            floating_group,
            label: panel_label(&self.core_layout, panel).unwrap_or("Панель"),
        }
    }

    fn group_drag_payload(
        &self,
        panel: PanelId,
        group: NodeId,
        group_rect: Rect,
        mouse_position_before: gpui::Point<gpui::Pixels>,
        drag_viewport_size: Size,
    ) -> GroupDragPayload {
        let floating = floating_group_for_panel(&self.core_layout, panel);
        let (floating_group, bounds_before) = match floating {
            Some((floating_group, bounds)) => (Some(floating_group), bounds),
            None => (None, group_rect),
        };

        GroupDragPayload {
            layout_before: self.core_layout.clone(),
            group,
            drag_start_position: Rc::new(Cell::new(mouse_position_before)),
            drag_viewport_size,
            bounds_before,
            floating_group,
            whole_area: false,
        }
    }

    /// Creates a drag payload from the first header of a main dock area.
    /// The payload references the area's root so every column and descendant moves together.
    fn dock_tree_drag_payload(
        &self,
        header_group: NodeId,
        rects: &dock_core::LayoutRects,
        mouse_position_before: gpui::Point<gpui::Pixels>,
        drag_viewport_size: Size,
    ) -> Option<GroupDragPayload> {
        for side in [DockSide::Left, DockSide::Right] {
            let Some(area) = self.core_layout.main_area(side) else {
                continue;
            };
            if dock_tree_header_group(&self.core_layout, side) != Some(header_group) {
                continue;
            }
            let root = area.root;
            let Some(bounds_before) = dock_area_bounds(&self.core_layout, root, rects) else {
                continue;
            };

            return Some(GroupDragPayload {
                layout_before: self.core_layout.clone(),
                group: root,
                drag_start_position: Rc::new(Cell::new(mouse_position_before)),
                drag_viewport_size,
                bounds_before,
                floating_group: None,
                whole_area: true,
            });
        }
        None
    }

    fn update_tab_from_drag(
        &mut self,
        event: &DragMoveEvent<TabDragPayload>,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        if self.active_tab_drag_payload.is_none() {
            self.active_tab_drag_payload = Some(drag.clone());
        }

        let current_pointer = Point::new(
            event.event.position.x.as_f32(),
            event.event.position.y.as_f32(),
        );
        let drag_start_position = drag.drag_start_position.get();
        let pointer_before = Point::new(
            drag_start_position.x.as_f32(),
            drag_start_position.y.as_f32(),
        );
        let movement =
            clamped_pointer_delta(current_pointer, pointer_before, drag.drag_viewport_size);
        let dx = movement.x;
        let dy = movement.y;
        if !dx.is_finite() || !dy.is_finite() || (dx.abs() < 0.01 && dy.abs() < 0.01) {
            return;
        }

        // Move by the pointer delta without mutating a docked source. The preview
        // remains outside Core's tree until an actual release commits the command.
        let origin = Point::new(
            drag.bounds_before.origin.x + dx,
            drag.bounds_before.origin.y + dy,
        );

        let source_has_other_tabs = group_has_multiple_tabs(&drag.layout_before, drag.group);
        if let Some(floating_group) = drag.floating_group.filter(|_| !source_has_other_tabs) {
            // When a floating group has only one tab, dragging that tab moves the
            // window. Tabs from multi-tab floating groups use the detach preview below.
            self.tab_drag_preview = None;
            self.core_layout = drag.layout_before.clone();
            if let Err(error) = self.core_layout.apply(
                DockCommand::MoveFloating {
                    group: floating_group,
                    origin,
                },
                &self.core_metrics,
            ) {
                eprintln!("Could not move floating tab group: {error:?}");
                self.core_layout = drag.layout_before.clone();
            }
        } else {
            let source_has_header = group_has_header(&drag.layout_before, drag.group);
            let (preview_bounds, _) = floating_drag_bounds(
                drag.bounds_before,
                movement,
                source_has_header,
                &self.core_metrics,
            );
            self.tab_drag_preview = Some(TabDragPreviewState {
                layout: drag.layout_before.clone(),
                group: drag.group,
                label: drag.label,
                bounds: preview_bounds,
            });
        }

        cx.notify();
    }

    fn update_group_from_drag(
        &mut self,
        event: &DragMoveEvent<GroupDragPayload>,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        if self.active_group_drag_payload.is_none() {
            self.active_group_drag_payload = Some(drag.clone());
        }

        let current_pointer = Point::new(
            event.event.position.x.as_f32(),
            event.event.position.y.as_f32(),
        );
        let drag_start_position = drag.drag_start_position.get();
        let pointer_before = Point::new(
            drag_start_position.x.as_f32(),
            drag_start_position.y.as_f32(),
        );
        let movement =
            clamped_pointer_delta(current_pointer, pointer_before, drag.drag_viewport_size);
        if !movement.x.is_finite() || !movement.y.is_finite() {
            return;
        }

        let origin = Point::new(
            drag.bounds_before.origin.x + movement.x,
            drag.bounds_before.origin.y + movement.y,
        );

        if let Some(floating_group) = drag.floating_group {
            // Already-floating groups keep moving live using Core's move command.
            self.group_drag_preview = None;
            self.core_layout = drag.layout_before.clone();
            if let Err(error) = self.core_layout.apply(
                DockCommand::MoveFloating {
                    group: floating_group,
                    origin,
                },
                &self.core_metrics,
            ) {
                eprintln!("Could not move floating group: {error:?}");
                self.core_layout = drag.layout_before.clone();
            }
        } else {
            let source_has_header = group_has_header(&drag.layout_before, drag.group);
            let (preview_bounds, _) = group_drag_bounds(
                drag.bounds_before,
                movement,
                source_has_header,
                &self.core_metrics,
            );
            self.group_drag_preview = Some(GroupDragPreviewState {
                layout: drag.layout_before.clone(),
                group: drag.group,
                bounds: preview_bounds,
            });
        }

        cx.notify();
    }

    /// Commits moving a whole group when the pointer is released.
    fn finish_group_drag(
        &mut self,
        drag: &GroupDragPayload,
        mouse_position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let current_pointer = Point::new(mouse_position.x.as_f32(), mouse_position.y.as_f32());
        let drag_start_position = drag.drag_start_position.get();
        let pointer_before = Point::new(
            drag_start_position.x.as_f32(),
            drag_start_position.y.as_f32(),
        );
        let movement =
            clamped_pointer_delta(current_pointer, pointer_before, drag.drag_viewport_size);

        if !movement.x.is_finite() || !movement.y.is_finite() {
            self.group_drag_preview = None;
            self.active_group_drag_payload = None;
            cx.notify();
            return;
        }

        let origin = Point::new(
            drag.bounds_before.origin.x + movement.x,
            drag.bounds_before.origin.y + movement.y,
        );
        self.core_layout = drag.layout_before.clone();

        let command = if let Some(floating_group) = drag.floating_group {
            DockCommand::MoveFloating {
                group: floating_group,
                origin,
            }
        } else {
            let source_has_header = group_has_header(&drag.layout_before, drag.group);
            let (_, floating_bounds) = group_drag_bounds(
                drag.bounds_before,
                movement,
                source_has_header,
                &self.core_metrics,
            );
            docked_group_float_command(&drag.layout_before, drag.group, floating_bounds)
        };

        if let Err(error) = self.core_layout.apply(command, &self.core_metrics) {
            eprintln!("Could not finish dragged group: {error:?}");
            self.core_layout = drag.layout_before.clone();
        }

        self.group_drag_preview = None;
        self.active_group_drag_payload = None;
        self.tab_drag_preview = None;
        self.active_tab_drag_payload = None;
        cx.notify();
    }

    /// Commits a tab detach when the pointer is released. At this stage the
    /// window root is the fallback drop target; explicit dock zones come later.
    fn finish_tab_drag(
        &mut self,
        drag: &TabDragPayload,
        mouse_position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let current_pointer = Point::new(
            mouse_position.x.as_f32(),
            mouse_position.y.as_f32(),
        );
        let drag_start_position = drag.drag_start_position.get();
        let pointer_before = Point::new(
            drag_start_position.x.as_f32(),
            drag_start_position.y.as_f32(),
        );
        let movement =
            clamped_pointer_delta(current_pointer, pointer_before, drag.drag_viewport_size);
        let dx = movement.x;
        let dy = movement.y;

        if !dx.is_finite() || !dy.is_finite() {
            self.tab_drag_preview = None;
            self.active_tab_drag_payload = None;
            cx.notify();
            return;
        }

        let origin = Point::new(
            drag.bounds_before.origin.x + dx,
            drag.bounds_before.origin.y + dy,
        );

        // Rebuild from the drag-start snapshot and apply one semantic command only
        // after release, so intermediate pointer movement never changes the tree.
        self.core_layout = drag.layout_before.clone();
        let source_has_other_tabs = group_has_multiple_tabs(&drag.layout_before, drag.group);
        let command = if let Some(floating_group) =
            drag.floating_group.filter(|_| !source_has_other_tabs)
        {
            DockCommand::MoveFloating {
                group: floating_group,
                origin,
            }
        } else {
            let source_has_header = group_has_header(&drag.layout_before, drag.group);
            let (_, floating_bounds) = floating_drag_bounds(
                drag.bounds_before,
                movement,
                source_has_header,
                &self.core_metrics,
            );
            docked_tab_float_command(
                &drag.layout_before,
                drag.panel,
                drag.group,
                floating_bounds,
            )
        };

        if let Err(error) = self.core_layout.apply(command, &self.core_metrics) {
            eprintln!("Could not finish dragged tab: {error:?}");
            self.core_layout = drag.layout_before.clone();
        }

        self.tab_drag_preview = None;
        self.active_tab_drag_payload = None;
        self.group_drag_preview = None;
        self.active_group_drag_payload = None;
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

            // Whole-group dragging keeps the Core layout intact and leaves a
            // placeholder at the group's original geometry until release.
            let dragged_group = self
                .active_group_drag_payload
                .as_ref()
                .filter(|drag| drag.floating_group.is_none() && drag.group == group_id);
            if dragged_group.is_some() {
                let placeholder = tabs_group::render(
                    &self.core_layout,
                    group_id,
                    self.theme,
                    0,
                    group_rect.rect.size.width,
                    group_rect.rect.size.height,
                    "",
                    None,
                    None,
                    None,
                    |_, _, _| {},
                    |_, _, _| {},
                    tabs_group::GroupRenderMode::SourcePlaceholder,
                    None,
                );
                root = root.child(
                    div()
                        .absolute()
                        .left(px(group_rect.rect.origin.x))
                        .top(px(group_rect.rect.origin.y))
                        .w(px(group_rect.rect.size.width))
                        .h(px(group_rect.rect.size.height))
                        .child(placeholder),
                );
                continue;
            }

            // A docked tab drag only hides its own source tab.
            let dragged_panel = self
                .active_tab_drag_payload
                .as_ref()
                .filter(|drag| drag.floating_group.is_none() && drag.group == group_id)
                .map(|drag| drag.panel);
            let Some(tabs_view) =
                group_tabs_view_excluding(&self.core_layout, group_id, dragged_panel)
            else {
                if dragged_panel.is_some() {
                    let placeholder = tabs_group::render(
                        &self.core_layout,
                        group_id,
                        self.theme,
                        0,
                        group_rect.rect.size.width,
                        group_rect.rect.size.height,
                        "",
                        None,
                        None,
                        None,
                        |_, _, _| {},
                        |_, _, _| {},
                        tabs_group::GroupRenderMode::SourcePlaceholder,
                        None,
                    );
                    root = root.child(
                        div()
                            .absolute()
                            .left(px(group_rect.rect.origin.x))
                            .top(px(group_rect.rect.origin.y))
                            .w(px(group_rect.rect.size.width))
                            .h(px(group_rect.rect.size.height))
                            .child(placeholder),
                    );
                }
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
            let group_drag_payload = Some(self.group_drag_payload(
                tabs_view.first_panel,
                group_id,
                group_rect.rect,
                drag_mouse_position,
                drag_viewport_size,
            ));

            let group_element = tabs_group::render(
                &self.core_layout,
                group_id,
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
                tabs_group::GroupRenderMode::Layout,
                group_drag_payload,
            );

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
            // When one tab is dragged out of a floating multi-tab group, render
            // the remaining source tab in place until the detach commits.
            let dragged_panel = self
                .active_tab_drag_payload
                .as_ref()
                .filter(|drag| {
                    drag.floating_group == Some(floating_rect.group)
                        && drag.group == group_id
                        && group_has_multiple_tabs(&drag.layout_before, drag.group)
                })
                .map(|drag| drag.panel);
            let Some(tabs_view) =
                group_tabs_view_excluding(&self.core_layout, group_id, dragged_panel)
            else {
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
            let group_drag_payload = Some(self.group_drag_payload(
                tabs_view.first_panel,
                group_id,
                group_rect.rect,
                drag_mouse_position,
                drag_viewport_size,
            ));

            let floating_bounds =
                floating_group_bounds(&self.core_layout, floating_rect.group)
                    .unwrap_or(floating_rect.rect);
            // Keep the model's raw origin while dragging. Core's geometry snapshot
            // clamps the visible rect for layout calculations; using that clamped
            // origin for painting made the panel stop before the pointer reached
            // the app-window edge.
            let floating_element = tabs_group::render(
                &self.core_layout,
                group_id,
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
                tabs_group::GroupRenderMode::Layout,
                group_drag_payload,
            );

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

        if let Some(preview) = &self.tab_drag_preview {
            let preview_element = tabs_group::render(
                &preview.layout,
                preview.group,
                self.theme,
                0,
                preview.bounds.size.width,
                preview.bounds.size.height,
                preview.label,
                None,
                None,
                None,
                |_, _, _| {},
                |_, _, _| {},
                tabs_group::GroupRenderMode::DragPreview,
                None,
            );
            root = root.child(
                div()
                    .absolute()
                    .left(px(preview.bounds.origin.x))
                    .top(px(preview.bounds.origin.y))
                    .w(px(preview.bounds.size.width))
                    .h(px(preview.bounds.size.height))
                    .child(preview_element),
            );
        }

        if let Some(preview) = &self.group_drag_preview {
            if let Some(tabs_view) = group_tabs_view(&preview.layout, preview.group) {
                let preview_element = tabs_group::render(
                    &preview.layout,
                    preview.group,
                    self.theme,
                    tabs_view.active_tab,
                    preview.bounds.size.width,
                    preview.bounds.size.height,
                    tabs_view.first_label,
                    tabs_view.second_label,
                    None,
                    None,
                    |_, _, _| {},
                    |_, _, _| {},
                    tabs_group::GroupRenderMode::GroupDragPreview,
                    None,
                );
                root = root.child(
                    div()
                        .absolute()
                        .left(px(preview.bounds.origin.x))
                        .top(px(preview.bounds.origin.y))
                        .w(px(preview.bounds.size.width))
                        .h(px(preview.bounds.size.height))
                        .child(preview_element),
                );
            }
        }

        root
            .on_drag_move::<TabDragPayload>(cx.listener(
                |this, event: &DragMoveEvent<TabDragPayload>, _window, cx| {
                    this.update_tab_from_drag(event, cx);
                },
            ))
            .on_drag_move::<GroupDragPayload>(cx.listener(
                |this, event: &DragMoveEvent<GroupDragPayload>, _window, cx| {
                    this.update_group_from_drag(event, cx);
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
            .on_drop(cx.listener(
                |this, drag: &TabDragPayload, window, cx| {
                    this.finish_tab_drag(drag, window.mouse_position(), cx);
                },
            ))
            .on_drop(cx.listener(
                |this, drag: &GroupDragPayload, window, cx| {
                    this.finish_group_drag(drag, window.mouse_position(), cx);
                },
            ))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| {
                    if let Some(drag) = this.active_group_drag_payload.clone() {
                        this.finish_group_drag(&drag, window.mouse_position(), cx);
                    } else if let Some(drag) = this.active_tab_drag_payload.clone() {
                        this.finish_tab_drag(&drag, window.mouse_position(), cx);
                    } else if this.group_drag_preview.take().is_some()
                        || this.tab_drag_preview.take().is_some()
                    {
                        cx.notify();
                    }
                }),
            )
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

// Floating composition always gets its own header. In the dock, the top group of
// a column owns the column header; groups below it start beneath a horizontal splitter.
// Derive both decisions from the current tree instead of maintaining parallel UI state.
fn group_has_header(layout: &Layout, group_id: NodeId) -> bool {
    if group_is_floating(layout, group_id) {
        return true;
    }

    layout
        .nodes()
        .find_map(|(_, node)| {
            let Node::Column(column) = node else {
                return None;
            };
            column
                .groups
                .iter()
                .position(|candidate| *candidate == group_id)
                .map(|index| index == 0)
        })
        .unwrap_or(true)
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

fn floating_drag_bounds(
    source_bounds: Rect,
    movement: Point,
    source_has_header: bool,
    metrics: &DockMetrics,
) -> (Rect, Rect) {
    let floating_size = floating_size_for_detach(source_bounds, source_has_header, metrics);
    let header_height = metrics.header_height.max(1.0);
    let source_header_height = if source_has_header {
        header_height
    } else {
        0.0
    };

    // Keep the tab strip at the same pointer-relative location before and after
    // release. During preview the tab strip is at the top; after release, the
    // floating header is inserted above it without changing the bottom edge.
    let preview_bounds = Rect::new(
        Point::new(
            source_bounds.origin.x + movement.x,
            source_bounds.origin.y + movement.y + source_header_height,
        ),
        Size::new(
            floating_size.width,
            (floating_size.height - header_height).max(1.0),
        ),
    );
    let floating_bounds = Rect::new(
        Point::new(
            source_bounds.origin.x + movement.x,
            source_bounds.origin.y + movement.y + source_header_height - header_height,
        ),
        floating_size,
    );

    (preview_bounds, floating_bounds)
}

/// Returns (headerless_preview_bounds, final_floating_bounds).
/// The preview and final window share their bottom edge and tab-strip position,
/// so adding the floating header on release does not make the group jump.
fn group_drag_bounds(
    source_bounds: Rect,
    movement: Point,
    source_has_header: bool,
    metrics: &DockMetrics,
) -> (Rect, Rect) {
    let header_height = metrics.header_height.max(1.0);
    let moved_source = Rect::new(
        Point::new(
            source_bounds.origin.x + movement.x,
            source_bounds.origin.y + movement.y,
        ),
        source_bounds.size,
    );

    if source_has_header {
        let preview_bounds = Rect::new(
            Point::new(
                moved_source.origin.x,
                moved_source.origin.y + header_height,
            ),
            Size::new(
                moved_source.size.width,
                (moved_source.size.height - header_height).max(1.0),
            ),
        );
        (preview_bounds, moved_source)
    } else {
        let floating_bounds = Rect::new(
            Point::new(
                moved_source.origin.x,
                moved_source.origin.y - header_height,
            ),
            Size::new(
                moved_source.size.width,
                moved_source.size.height + header_height,
            ),
        );
        (moved_source, floating_bounds)
    }
}

fn floating_size_for_detach(
    source_bounds: Rect,
    has_header: bool,
    metrics: &DockMetrics,
) -> Size {
    let source_header_height = if has_header {
        metrics.header_height.max(1.0)
    } else {
        0.0
    };
    let floating_header_height = metrics.header_height.max(1.0);
    let tab_strip_height = metrics.tab_strip_height.max(1.0);
    let minimum_content_height = metrics.min_group_height.max(1.0);
    let source_content_height =
        (source_bounds.size.height - source_header_height - tab_strip_height)
            .max(minimum_content_height);
    let floating_content_height =
        (source_content_height * FLOATING_HEIGHT_RATIO).max(minimum_content_height);

    // Floating groups always render with a header, even when their docked source
    // was below a splitter and therefore had no separate header.
    let minimum_floating_height =
        floating_header_height + tab_strip_height + minimum_content_height;
    let height = (floating_header_height + tab_strip_height + floating_content_height)
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

fn panel_label(layout: &Layout, panel_id: PanelId) -> Option<&'static str> {
    let instance = layout.panel_instance(panel_id)?;
    Some(match instance.key.as_str() {
        "layers" => "Вкладка 1",
        "color" => "Вкладка 2",
        "properties" => "Вкладка 3",
        "history" => "Вкладка 4",
        _ => "Панель",
    })
}

fn docked_tab_float_command(
    layout: &Layout,
    panel: PanelId,
    group: NodeId,
    bounds: Rect,
) -> DockCommand {
    let source_tab_count = match layout.node(group) {
        Some(Node::Tabs(tabs)) => tabs.children.len(),
        _ => 0,
    };

    if source_tab_count > 1 {
        DockCommand::FloatPanelAsGroup { panel, bounds }
    } else if let Some((column, group_count)) = column_containing_group(layout, group) {
        if group_count > 1 {
            DockCommand::FloatGroupAsColumn { group, bounds }
        } else {
            // Float the containing column rather than leaving an invalid empty one.
            DockCommand::Float { node: column, bounds }
        }
    } else {
        DockCommand::Float { node: group, bounds }
    }
}

fn docked_group_float_command(layout: &Layout, group: NodeId, bounds: Rect) -> DockCommand {
    if let Some((column, group_count)) = column_containing_group(layout, group) {
        if group_count > 1 {
            DockCommand::FloatGroupAsColumn { group, bounds }
        } else {
            // Float the containing column to avoid leaving an empty column behind.
            DockCommand::Float { node: column, bounds }
        }
    } else {
        DockCommand::Float { node: group, bounds }
    }
}

fn group_has_multiple_tabs(layout: &Layout, group_id: NodeId) -> bool {
    matches!(layout.node(group_id), Some(Node::Tabs(tabs)) if tabs.children.len() > 1)
}

/// Returns drag bounds whose origin corresponds to the tab slot being dragged.
/// A detached preview contains only one tab, so a source tab at index 1 must
/// shift the preview one tab-width to the right to keep the cursor over that tab.
fn tab_drag_bounds_for_panel(
    layout: &Layout,
    group_id: NodeId,
    panel_id: PanelId,
    group_bounds: Rect,
    tab_width: f32,
) -> Rect {
    let tab_index = match layout.node(group_id) {
        Some(Node::Tabs(tabs)) => tabs.children.iter().position(|child_id| {
            matches!(
                layout.node(*child_id),
                Some(Node::Panel(panel)) if panel.panel_id == panel_id
            )
        }),
        _ => None,
    }
    .unwrap_or(0);
    let offset_x = tab_index as f32 * tab_width.max(1.0);

    Rect::new(
        Point::new(group_bounds.origin.x + offset_x, group_bounds.origin.y),
        group_bounds.size,
    )
}

fn group_tabs_view(layout: &Layout, group_id: NodeId) -> Option<GroupTabsView> {
    group_tabs_view_excluding(layout, group_id, None)
}

fn group_tabs_view_excluding(
    layout: &Layout,
    group_id: NodeId,
    excluded_panel: Option<PanelId>,
) -> Option<GroupTabsView> {
    let Some(Node::Tabs(tabs)) = layout.node(group_id) else {
        return None;
    };
    let mut children = tabs
        .children
        .iter()
        .filter_map(|child_id| {
            let Some(Node::Panel(panel)) = layout.node(*child_id) else {
                return None;
            };
            if Some(panel.panel_id) == excluded_panel {
                return None;
            }
            let label = panel_label(layout, panel.panel_id)?;
            Some((panel.panel_id, label))
        })
        .take(2);

    let (first_panel, first_label) = children.next()?;
    let second = children.next();
    Some(GroupTabsView {
        active_tab: if excluded_panel.is_some() {
            0
        } else {
            tabs.active_index.min(if second.is_some() { 1 } else { 0 })
        },
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
        group_tabs_view_excluding, tab_drag_bounds_for_panel,
        clamped_pointer_delta, panel_is_open, toggle_core_panel, floating_size_for_detach,
        floating_drag_bounds, group_drag_bounds, docked_group_float_command,
        FLOATING_HEIGHT_RATIO,
    };
    use dock_core::{Axis, DockCommand, DockSide, Node, Point, Rect, Size};

    #[test]
    fn dragging_tab_hides_it_from_source_and_keeps_only_remaining_tab() {
        let theme = ui::Theme::default();
        let (layout, _, _, upper_group, lower_group) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let Node::Tabs(source_tabs) = layout.node(upper_group).unwrap() else {
            panic!("upper group should be a tab group");
        };
        assert_eq!(source_tabs.children.len(), 2);

        let dragged_panel = match layout.node(source_tabs.children[0]).unwrap() {
            Node::Panel(panel) => panel.panel_id,
            _ => panic!("tab child should be a Panel"),
        };
        let remaining = group_tabs_view_excluding(&layout, upper_group, Some(dragged_panel))
            .expect("the source group should keep the tab that was not dragged");
        assert!(remaining.second_panel.is_none());
        assert_ne!(remaining.first_panel, dragged_panel);
        assert_eq!(remaining.active_tab, 0);

        let Node::Tabs(lower_tabs) = layout.node(lower_group).unwrap() else {
            panic!("lower group should be a tab group");
        };
        assert_eq!(lower_tabs.children.len(), 1);
        let only_panel = match layout.node(lower_tabs.children[0]).unwrap() {
            Node::Panel(panel) => panel.panel_id,
            _ => panic!("tab child should be a Panel"),
        };
        assert!(
            group_tabs_view_excluding(&layout, lower_group, Some(only_panel)).is_none(),
            "a group whose only tab is dragged should leave a placeholder"
        );
    }

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
        let floating_tabs = floating_root_tabs_group(&layout, floating.0).unwrap();
        assert!(group_is_floating(&layout, floating_tabs));
        assert!(group_has_header(&layout, floating_tabs));
    }

    #[test]
    fn lower_docked_group_has_no_header_but_floating_group_does() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, _, headered_group, headerless_group) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let history = panel_id_for_key(&layout, "history").unwrap();

        assert!(group_has_header(&layout, headered_group));
        assert!(!group_has_header(&layout, headerless_group));
        assert_eq!(group_containing_panel(&layout, history), Some(headerless_group));

        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        let source_bounds = rects
            .groups
            .iter()
            .find(|entry| entry.group == headerless_group)
            .expect("lower History group should have geometry")
            .rect;
        let source_has_header = group_has_header(&layout, headerless_group);
        let size = floating_size_for_detach(source_bounds, source_has_header, &metrics);
        let bounds = Rect::new(
            Point::new(source_bounds.origin.x + 20.0, source_bounds.origin.y + 16.0),
            size,
        );

        layout
            .apply(
                DockCommand::FloatGroupAsColumn {
                    group: headerless_group,
                    bounds,
                },
                &metrics,
            )
            .expect("the lower History group should detach as floating");
        assert_eq!(layout.validate(&metrics), Ok(()));

        let (floating_id, _) = floating_group_for_panel(&layout, history)
            .expect("History should now belong to a floating composition");
        let floating_tabs = floating_root_tabs_group(&layout, floating_id)
            .expect("floating composition should contain the History Tabs node");

        assert!(group_is_floating(&layout, floating_tabs));
        assert!(group_has_header(&layout, floating_tabs));
    }

    #[test]
    fn remaining_group_gets_header_when_it_becomes_the_only_group_in_column() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, _, upper_group, lower_group) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let history = panel_id_for_key(&layout, "history").unwrap();
        assert!(!group_has_header(&layout, lower_group));

        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        let source_bounds = rects
            .groups
            .iter()
            .find(|entry| entry.group == upper_group)
            .expect("upper group should have geometry")
            .rect;
        let bounds = Rect::new(
            Point::new(source_bounds.origin.x + 20.0, source_bounds.origin.y + 16.0),
            floating_size_for_detach(source_bounds, true, &metrics),
        );

        layout
            .apply(
                DockCommand::FloatGroupAsColumn {
                    group: upper_group,
                    bounds,
                },
                &metrics,
            )
            .expect("upper group should float and leave History in the dock");
        assert_eq!(layout.validate(&metrics), Ok(()));

        let (_, remaining_group_count) = column_containing_group(&layout, lower_group)
            .expect("History should remain in its original column");
        assert_eq!(remaining_group_count, 1);
        assert_eq!(group_containing_panel(&layout, history), Some(lower_group));
        assert!(group_has_header(&layout, lower_group));
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
        let expected_without_header = (header_height
            + tab_strip_height
            + (source_content_without_header * FLOATING_HEIGHT_RATIO).max(minimum_content_height))
            .max(minimum_floating_height);
        assert!((without_header.height - expected_without_header).abs() < 0.001);
    }

    #[test]
    fn dragging_group_preserves_all_tabs_and_keeps_core_layout_valid() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, _, group, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let initial_tab_count = match layout.node(group).unwrap() {
            Node::Tabs(tabs) => tabs.children.len(),
            _ => panic!("the source should be a tab group"),
        };
        assert_eq!(initial_tab_count, 2);

        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        let source_bounds = rects
            .groups
            .iter()
            .find(|entry| entry.group == group)
            .expect("the group should have a source rectangle")
            .rect;
        let (_, bounds) = group_drag_bounds(
            source_bounds,
            Point::new(40.0, 25.0),
            group_has_header(&layout, group),
            &metrics,
        );

        let command = docked_group_float_command(&layout, group, bounds);
        layout
            .apply(command, &metrics)
            .expect("dragging a group should float the whole group");
        assert_eq!(layout.validate(&metrics), Ok(()));

        let floating_id = layout
            .floating_groups()
            .next()
            .map(|(id, _)| id)
            .expect("the group should now be floating");
        let floating_tabs_group = floating_root_tabs_group(&layout, floating_id)
            .expect("the floating composition should retain a tab group");
        let Node::Tabs(floating_tabs) = layout.node(floating_tabs_group).unwrap() else {
            panic!("the floating root should be a tab group");
        };
        assert_eq!(floating_tabs.children.len(), initial_tab_count);
    }

    #[test]
    fn group_drag_preview_is_headerless_and_keeps_pointer_side_of_tab_strip_aligned() {
        let theme = ui::Theme::default();
        let (_, metrics, _, _, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let source = Rect::new(Point::new(100.0, 150.0), Size::new(320.0, 420.0));
        let movement = Point::new(25.0, 35.0);
        let header_height = metrics.header_height.max(1.0);

        for source_has_header in [false, true] {
            let (preview, floating) =
                group_drag_bounds(source, movement, source_has_header, &metrics);

            assert_eq!(preview.origin.x, source.origin.x + movement.x);
            assert_eq!(floating.origin.x, preview.origin.x);
            assert!(
                (preview.origin.y - (floating.origin.y + header_height)).abs() < 0.001,
                "the final floating header must be inserted above the headerless preview"
            );
            assert!(
                ((preview.origin.y + preview.size.height)
                    - (floating.origin.y + floating.size.height))
                    .abs()
                    < 0.001,
                "the bottom edge must stay still when the header appears"
            );
            assert_eq!(preview.size.width, source.size.width);
            assert_eq!(floating.size.width, source.size.width);

            if source_has_header {
                assert_eq!(preview.origin.y, source.origin.y + movement.y + header_height);
                assert_eq!(preview.size.height, source.size.height - header_height);
                assert_eq!(floating.origin.y, source.origin.y + movement.y);
                assert_eq!(floating.size.height, source.size.height);
            } else {
                assert_eq!(preview.origin.y, source.origin.y + movement.y);
                assert_eq!(preview.size.height, source.size.height);
                assert_eq!(floating.origin.y, source.origin.y + movement.y - header_height);
                assert_eq!(floating.size.height, source.size.height + header_height);
            }
        }
    }

    #[test]
    fn headerless_drag_preview_and_final_window_keep_the_tab_strip_aligned() {
        let theme = ui::Theme::default();
        let (_, metrics, _, _, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let source_bounds = Rect::new(Point::new(100.0, 150.0), Size::new(320.0, 500.0));
        let movement = Point::new(37.0, 29.0);
        let header_height = metrics.header_height.max(1.0);

        for source_has_header in [false, true] {
            let (preview, floating) =
                floating_drag_bounds(source_bounds, movement, source_has_header, &metrics);

            assert!(
                (preview.origin.y - (floating.origin.y + header_height)).abs() < 0.001,
                "the final header must be inserted above the preview tab strip"
            );
            assert!((preview.size.width - floating.size.width).abs() < 0.001);
            assert!(
                (preview.size.height + header_height - floating.size.height).abs() < 0.001,
                "the preview reserves exactly the header height"
            );
            assert!(
                ((preview.origin.y + preview.size.height)
                    - (floating.origin.y + floating.size.height))
                    .abs()
                    < 0.001,
                "the bottom edge should not jump when the header appears"
            );
        }
    }

    #[test]
    fn detached_second_tab_keeps_its_source_slot_under_the_pointer() {
        let theme = ui::Theme::default();
        let (layout, _, _, group, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let Node::Tabs(tabs) = layout.node(group).unwrap() else {
            panic!("source should be a Tabs group");
        };
        assert_eq!(tabs.children.len(), 2);

        let panel_at = |index: usize| match layout.node(tabs.children[index]).unwrap() {
            Node::Panel(panel) => panel.panel_id,
            _ => panic!("tabs children should be panels"),
        };
        let first_panel = panel_at(0);
        let second_panel = panel_at(1);
        let source_bounds = Rect::new(Point::new(80.0, 150.0), Size::new(320.0, 480.0));
        let tab_width = theme.metrics.column_tab_width.max(1.0);

        let first_bounds =
            tab_drag_bounds_for_panel(&layout, group, first_panel, source_bounds, tab_width);
        let second_bounds =
            tab_drag_bounds_for_panel(&layout, group, second_panel, source_bounds, tab_width);

        assert_eq!(first_bounds.origin.x, source_bounds.origin.x);
        assert_eq!(second_bounds.origin.x, source_bounds.origin.x + tab_width);
        assert_eq!(second_bounds.origin.y, source_bounds.origin.y);
        assert_eq!(second_bounds.size, source_bounds.size);
    }

    #[test]
    fn tab_can_be_detached_from_a_floating_multi_tab_group() {
        let theme = ui::Theme::default();
        let (mut layout, metrics, _, group, _) =
            build_core_layout(&theme).expect("initial Core layout should be valid");
        let panel_to_detach = panel_id_for_key(&layout, "color").unwrap();
        let panel_to_keep = panel_id_for_key(&layout, "properties").unwrap();

        let viewport = Rect::new(Point::ZERO, Size::new(1200.0, 768.0));
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        let source_bounds = rects
            .groups
            .iter()
            .find(|entry| entry.group == group)
            .expect("source group should have geometry")
            .rect;
        let first_floating_bounds = Rect::new(
            Point::new(source_bounds.origin.x + 30.0, source_bounds.origin.y + 25.0),
            floating_size_for_detach(source_bounds, true, &metrics),
        );

        layout
            .apply(
                DockCommand::FloatGroupAsColumn {
                    group,
                    bounds: first_floating_bounds,
                },
                &metrics,
            )
            .expect("the two-tab source group should float");
        assert_eq!(layout.validate(&metrics), Ok(()));

        let (source_floating_id, _) = floating_group_for_panel(&layout, panel_to_keep)
            .expect("the retained tab should belong to the floating source group");
        assert_eq!(
            floating_group_for_panel(&layout, panel_to_detach)
                .map(|(id, _)| id),
            Some(source_floating_id)
        );

        let detached_bounds = Rect::new(
            Point::new(
                first_floating_bounds.origin.x + 80.0,
                first_floating_bounds.origin.y + 40.0,
            ),
            floating_size_for_detach(first_floating_bounds, true, &metrics),
        );
        layout
            .apply(
                DockCommand::FloatPanelAsGroup {
                    panel: panel_to_detach,
                    bounds: detached_bounds,
                },
                &metrics,
            )
            .expect("a tab should be detachable from an already-floating group");
        assert_eq!(layout.validate(&metrics), Ok(()));
        assert_eq!(layout.floating_groups().count(), 2);

        let (retained_floating_id, _) = floating_group_for_panel(&layout, panel_to_keep)
            .expect("the retained tab should remain in the original floating window");
        let (detached_floating_id, _) = floating_group_for_panel(&layout, panel_to_detach)
            .expect("the extracted tab should have its own floating window");
        assert_eq!(retained_floating_id, source_floating_id);
        assert_ne!(detached_floating_id, source_floating_id);
        assert!(matches!(
            layout.node(group),
            Some(Node::Tabs(tabs)) if tabs.children.len() == 1
        ));
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
