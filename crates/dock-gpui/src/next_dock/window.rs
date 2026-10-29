//! Native window for the independent dock prototype.

use dock_core::{Point, Rect, Size};
use gpui::{Context, IntoElement, MouseButton, MouseMoveEvent, Render, Window};
use ui::Theme;

#[cfg(windows)]
use std::collections::HashMap;

#[cfg(windows)]
use gpui::WindowHandle;
#[cfg(windows)]
use platform::windows::NativeWindowHandle;

#[cfg(windows)]
use super::floating_window::FloatingCompositionWindow;
use super::geometry;
use super::model::{CompositionContainer, DockLayout, NodePathStep, Panel, SplitAxis, TabGroup};
use super::view::{self, SplitResizeSpec};

/// Owns the prototype layout and renders it in a regular GPUI window.
pub(super) struct NextDockWindow {
    theme: Theme,
    pub(super) container: CompositionContainer,
    active_resize: Option<ActiveResize>,
    pub(super) floating_compositions: Vec<FloatingComposition>,
    next_floating_id: u64,
    #[cfg(windows)]
    pub(super) native_main_handle: Option<NativeWindowHandle>,
    #[cfg(windows)]
    pub(super) native_main_viewport: Size,
    #[cfg(windows)]
    pub(super) native_floating_windows: HashMap<u64, WindowHandle<FloatingCompositionWindow>>,
    #[cfg(windows)]
    pub(super) native_floating_pool: Vec<WindowHandle<FloatingCompositionWindow>>,
    #[cfg(windows)]
    last_native_sync_ids: Option<Vec<u64>>,
    pending_move: Option<PendingCompositionMove>,
    active_move: Option<ActiveCompositionMove>,
    pub(super) composition_drop_zone: Option<CompositionDropZone>,
    pending_tab_drag: Option<PendingTabDrag>,
    active_tab_drag: Option<ActiveTabDrag>,
    pending_group_drag: Option<PendingGroupDrag>,
    active_group_drag: Option<ActiveGroupDrag>,
}

#[derive(Clone)]
struct ActiveResize {
    spec: SplitResizeSpec,
    start_position: f32,
}

#[derive(Clone)]
struct PendingCompositionMove {
    start_position: Point,
    viewport_size: Size,
    composition_index: usize,
}

#[derive(Clone)]
struct ActiveCompositionMove {
    start_position: Point,
    start_origin: Point,
    viewport_size: Size,
    composition_index: usize,
}

#[derive(Clone)]
struct PendingTabDrag {
    composition_index: usize,
    path: Vec<NodePathStep>,
    panel_index: usize,
    panel: Panel,
    start_position: Point,
    viewport_size: Size,
    strip_bounds: Rect,
    tab_widths: Vec<f32>,
    grab_offset_x: f32,
    grab_offset_y: f32,
    reorder_started: bool,
    preview_bounds: Option<Rect>,
    group_key: String,
}

#[derive(Clone)]
struct ActiveTabDrag {
    pending: PendingTabDrag,
    start_origin: Point,
    bounds: Rect,
}

#[derive(Clone)]
struct PendingGroupDrag {
    composition_index: usize,
    path: Vec<NodePathStep>,
    group: TabGroup,
    start_position: Point,
    viewport_size: Size,
    grab_offset_x: f32,
    grab_offset_y: f32,
    group_key: String,
}

#[derive(Clone)]
struct ActiveGroupDrag {
    pending: PendingGroupDrag,
    start_origin: Point,
    bounds: Rect,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TabDragPreview {
    pub(super) panel: Panel,
    pub(super) bounds: Rect,
}

#[derive(Clone, Debug)]
pub(super) struct GroupDragPreview {
    pub(super) group: TabGroup,
    pub(super) bounds: Rect,
    pub(super) composition_index: usize,
    pub(super) group_key: String,
}

#[derive(Clone, Debug)]
pub(super) struct TabReorderPreview {
    pub(super) panel: Panel,
    pub(super) bounds: Rect,
    pub(super) composition_index: usize,
    pub(super) group_key: String,
    pub(super) panel_index: usize,
}

#[derive(Clone)]
pub(super) struct TabDragContext {
    pub(super) composition_index: usize,
    pub(super) path: Vec<NodePathStep>,
    pub(super) viewport_size: Size,
    pub(super) owner: gpui::WeakEntity<NextDockWindow>,
    pub(super) strip_bounds: Rect,
    pub(super) group_key: String,
    pub(super) dragged_panel_index: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FloatingComposition {
    pub(super) id: u64,
    pub(super) composition_index: usize,
    pub(super) bounds: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompositionDropZone {
    LeftEdge,
    RightEdge,
}

pub(super) const COMPOSITION_DROP_ZONE_WIDTH: f32 = 24.0;

impl NextDockWindow {
    pub(super) fn new(theme: Theme) -> Self {
        Self {
            theme,
            container: CompositionContainer::initial(),
            active_resize: None,
            floating_compositions: Vec::new(),
            next_floating_id: 1,
            #[cfg(windows)]
            native_main_handle: None,
            #[cfg(windows)]
            native_main_viewport: Size::new(1.0, 1.0),
            #[cfg(windows)]
            native_floating_windows: HashMap::new(),
            #[cfg(windows)]
            native_floating_pool: Vec::new(),
            #[cfg(windows)]
            last_native_sync_ids: None,
            pending_move: None,
            active_move: None,
            composition_drop_zone: None,
            pending_tab_drag: None,
            active_tab_drag: None,
            pending_group_drag: None,
            active_group_drag: None,
        }
    }
    fn add_floating_composition(&mut self, composition_index: usize, bounds: Rect) -> u64 {
        let id = self.next_floating_id;
        self.next_floating_id = self.next_floating_id.wrapping_add(1);
        self.floating_compositions.push(FloatingComposition {
            id,
            composition_index,
            bounds,
        });
        id
    }

    pub(super) fn floating_composition_snapshot(
        &self,
        id: u64,
    ) -> Option<(FloatingComposition, DockLayout, Theme)> {
        let floating = self
            .floating_compositions
            .iter()
            .find(|floating| floating.id == id)
            .copied()?;
        let layout = self
            .container
            .slots
            .get(floating.composition_index)?
            .layout
            .as_ref()?
            .clone();
        Some((floating, layout, self.theme))
    }

    pub(super) fn clear_native_move_state(&mut self) {
        self.pending_move = None;
        self.active_move = None;
        self.composition_drop_zone = None;
    }

    pub(super) fn begin_resize(
        &mut self,
        spec: SplitResizeSpec,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.pending_tab_drag = None;
        self.active_tab_drag = None;
        self.pending_group_drag = None;
        self.active_group_drag = None;
        self.pending_move = None;
        self.active_move = None;

        if spec.available_extent <= f32::EPSILON
            || spec.boundary_index + 1 >= spec.initial_extents.len()
        {
            return;
        }

        let start_position = match spec.axis {
            SplitAxis::Horizontal => position.x.as_f32(),
            SplitAxis::Vertical => position.y.as_f32(),
        };

        self.active_resize = Some(ActiveResize {
            spec,
            start_position,
        });

        cx.notify();
    }

    pub(super) fn update_resize(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(session) = self.active_resize.clone() else {
            return;
        };

        if event.pressed_button != Some(MouseButton::Left) {
            self.active_resize = None;
            cx.notify();
            return;
        }

        let spec = &session.spec;

        let current_position = match spec.axis {
            SplitAxis::Horizontal => event.position.x.as_f32(),
            SplitAxis::Vertical => event.position.y.as_f32(),
        };

        let delta = current_position - session.start_position;
        if !delta.is_finite() {
            return;
        }

        let index = spec.boundary_index;
        let pair_extent = spec.initial_extents[index] + spec.initial_extents[index + 1];

        if pair_extent <= f32::EPSILON {
            return;
        }

        let min_second = spec.min_second.max(0.0).min(pair_extent);
        let max_first = (pair_extent - min_second).max(0.0);
        let min_first = spec.min_first.max(0.0).min(max_first);

        let first_extent = (spec.initial_extents[index] + delta).clamp(min_first, max_first);
        let second_extent = pair_extent - first_extent;

        let mut resized_extents = spec.initial_extents.clone();
        resized_extents[index] = first_extent;
        resized_extents[index + 1] = second_extent;

        let weights: Vec<f32> = resized_extents
            .iter()
            .map(|extent| extent.max(0.0) / spec.available_extent)
            .collect();

        let Some(layout) = self.container.layout_mut(spec.composition_index) else {
            self.active_resize = None;
            return;
        };

        let Some(current_weights) = layout.split_weights_mut(&spec.path) else {
            self.active_resize = None;
            return;
        };

        if current_weights.len() != weights.len() {
            self.active_resize = None;
            return;
        }

        *current_weights = weights;
        cx.notify();
    }

    pub(super) fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.active_resize.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn begin_tab_drag(
        &mut self,
        composition_index: usize,
        path: Vec<NodePathStep>,
        panel_index: usize,
        panel: Panel,
        position: gpui::Point<gpui::Pixels>,
        viewport_size: Size,
        strip_bounds: Rect,
        tab_widths: Vec<f32>,
        grab_offset_x: f32,
        group_key: String,
    ) {
        if self.active_resize.is_some() {
            return;
        }

        self.pending_move = None;
        self.active_move = None;
        self.composition_drop_zone = None;
        self.active_tab_drag = None;
        self.pending_group_drag = None;
        self.active_group_drag = None;

        self.pending_tab_drag = Some(PendingTabDrag {
            composition_index,
            path,
            panel_index,
            panel,
            start_position: Point::new(
                position.x.as_f32(),
                (position.y.as_f32() - geometry::HEADER_HEIGHT).max(0.0),
            ),
            viewport_size: Size::new(viewport_size.width.max(1.0), viewport_size.height.max(1.0)),
            strip_bounds,
            tab_widths,
            grab_offset_x,
            grab_offset_y: position.y.as_f32() - geometry::HEADER_HEIGHT - strip_bounds.origin.y,
            reorder_started: false,
            preview_bounds: None,
            group_key,
        });
    }

    pub(super) fn begin_group_drag(
        &mut self,
        composition_index: usize,
        path: Vec<NodePathStep>,
        group: TabGroup,
        position: gpui::Point<gpui::Pixels>,
        viewport_size: Size,
        strip_bounds: Rect,
        grab_offset_x: f32,
        group_key: String,
    ) {
        if self.active_resize.is_some() {
            return;
        }

        if self
            .floating_compositions
            .iter()
            .any(|floating| floating.composition_index == composition_index)
        {
            self.begin_composition_move(
                composition_index,
                position,
                viewport_size.width,
                viewport_size.height,
            );
            return;
        }

        self.pending_tab_drag = None;
        self.active_tab_drag = None;
        self.pending_move = None;
        self.active_move = None;
        self.composition_drop_zone = None;
        self.active_group_drag = None;

        self.pending_group_drag = Some(PendingGroupDrag {
            composition_index,
            path,
            group,
            start_position: Point::new(
                position.x.as_f32(),
                (position.y.as_f32() - geometry::HEADER_HEIGHT).max(0.0),
            ),
            viewport_size: Size::new(viewport_size.width.max(1.0), viewport_size.height.max(1.0)),
            grab_offset_x,
            grab_offset_y: position.y.as_f32() - geometry::HEADER_HEIGHT - strip_bounds.origin.y,
            group_key,
        });
    }

    pub(super) fn update_group_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }

        let pointer = Point::new(
            event.position.x.as_f32(),
            (event.position.y.as_f32() - geometry::HEADER_HEIGHT).max(0.0),
        );

        if let Some(active) = self.active_group_drag.clone() {
            let viewport = active.pending.viewport_size;
            let size = active.bounds.size;

            let origin = Point::new(
                (active.start_origin.x + pointer.x - active.pending.start_position.x)
                    .clamp(0.0, (viewport.width - size.width).max(0.0)),
                (active.start_origin.y + pointer.y - active.pending.start_position.y)
                    .clamp(0.0, (viewport.height - size.height).max(0.0)),
            );

            self.active_group_drag = Some(ActiveGroupDrag {
                bounds: Rect::new(origin, size),
                ..active
            });

            cx.notify();
            return;
        }

        let Some(mut pending) = self.pending_group_drag.take() else {
            return;
        };

        // A click alone should not detach the group, but dragging from
        // the dedicated group drag zone should float it without leaving the strip.
        let dx = pointer.x - pending.start_position.x;
        let dy = pointer.y - pending.start_position.y;
        if dx * dx + dy * dy < 25.0 {
            self.pending_group_drag = Some(pending);
            return;
        }

        let viewport = pending.viewport_size;
        let width = (viewport.width * 0.38)
            .min(480.0)
            .clamp(1.0, viewport.width);
        let height = (viewport.height * 0.55)
            .min(520.0)
            .clamp(1.0, viewport.height);
        let column_header_height = self.theme.metrics.column_header_height.max(0.0);

        let origin = Point::new(
            (pointer.x - pending.grab_offset_x).clamp(0.0, (viewport.width - width).max(0.0)),
            (pointer.y - column_header_height - pending.grab_offset_y)
                .clamp(0.0, (viewport.height - height).max(0.0)),
        );

        pending.start_position = pointer;
        self.active_group_drag = Some(ActiveGroupDrag {
            pending,
            start_origin: origin,
            bounds: Rect::new(origin, Size::new(width, height)),
        });

        // Promote the group while the mouse is still held. The native
        // floating surface is already prepared and can take over the drag.
        self.end_group_drag(cx);
    }

    pub(super) fn update_tab_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }

        let pointer = Point::new(
            event.position.x.as_f32(),
            (event.position.y.as_f32() - geometry::HEADER_HEIGHT).max(0.0),
        );

        // Если вкладка уже вышла из полосы и стала плавающим превью,
        // продолжаем обычное перетаскивание.
        if let Some(active) = self.active_tab_drag.clone() {
            let viewport = active.pending.viewport_size;
            let size = active.bounds.size;

            let origin = Point::new(
                (active.start_origin.x + pointer.x - active.pending.start_position.x)
                    .clamp(0.0, (viewport.width - size.width).max(0.0)),
                (active.start_origin.y + pointer.y - active.pending.start_position.y)
                    .clamp(0.0, (viewport.height - size.height).max(0.0)),
            );

            self.active_tab_drag = Some(ActiveTabDrag {
                bounds: Rect::new(origin, size),
                ..active
            });

            cx.notify();
            return;
        }

        let Some(mut pending) = self.pending_tab_drag.take() else {
            return;
        };

        let strip = pending.strip_bounds;

        let cursor_inside_strip = pointer.x >= strip.origin.x
            && pointer.x <= strip.origin.x + strip.size.width
            && pointer.y >= strip.origin.y
            && pointer.y <= strip.origin.y + strip.size.height;

        let dx = pointer.x - pending.start_position.x;
        let dy = pointer.y - pending.start_position.y;

        if cursor_inside_strip {
            // Клик или очень короткое движение ещё не начинает перестановку.
            if !pending.reorder_started && dx * dx + dy * dy < 25.0 {
                self.pending_tab_drag = Some(pending);
                return;
            }

            pending.reorder_started = true;

            let local_x = pointer.x - strip.origin.x;

            // Переставляем модель при пересечении порога соседней вкладки.
            // Вправо — после 40% ширины следующей вкладки, влево — до отметки 60% предыдущей.
            loop {
                let index = pending.panel_index;
                let mut moved = false;

                let tab_lane_width: f32 = pending.tab_widths.iter().sum();

                let dragged_width = pending.tab_widths.get(index).copied().unwrap_or(120.0);

                let dragged_left = (local_x - pending.grab_offset_x)
                    .clamp(0.0, (tab_lane_width - dragged_width).max(0.0));

                let dragged_center = dragged_left + dragged_width * 0.5;

                if index + 1 < pending.tab_widths.len() {
                    let next = index + 1;
                    let next_start: f32 = pending.tab_widths[..next].iter().sum();
                    let threshold = next_start + pending.tab_widths[next] * 0.40;

                    if dragged_center >= threshold {
                        moved = self
                            .container
                            .layout_mut(pending.composition_index)
                            .is_some_and(|layout| layout.reorder_panel(&pending.path, index, next));

                        if moved {
                            let width = pending.tab_widths.remove(index);
                            pending.tab_widths.insert(next, width);
                            pending.panel_index = next;
                        }
                    }
                }

                if moved {
                    continue;
                }

                if index > 0 {
                    let previous = index - 1;
                    let previous_start: f32 = pending.tab_widths[..previous].iter().sum();
                    let threshold = previous_start + pending.tab_widths[previous] * 0.60;

                    if dragged_center <= threshold {
                        moved = self
                            .container
                            .layout_mut(pending.composition_index)
                            .is_some_and(|layout| {
                                layout.reorder_panel(&pending.path, index, previous)
                            });

                        if moved {
                            let width = pending.tab_widths.remove(index);
                            pending.tab_widths.insert(previous, width);
                            pending.panel_index = previous;
                        }
                    }
                }

                if !moved {
                    break;
                }
            }

            // Рисуем перетаскиваемую вкладку поверх ряда.
            let tab_width = pending
                .tab_widths
                .get(pending.panel_index)
                .copied()
                .unwrap_or(120.0);

            // Превью может заходить в свободную drag-зону,
            // но не должно перекрывать кнопку меню.
            let menu_width = self.theme.metrics.group_control_button_width.max(0.0);

            let tab_area_width = (strip.size.width - menu_width).max(0.0);

            let tab_left =
                (local_x - pending.grab_offset_x).clamp(0.0, (tab_area_width - tab_width).max(0.0));

            pending.preview_bounds = Some(Rect::new(
                Point::new(strip.origin.x + tab_left, strip.origin.y),
                Size::new(tab_width, strip.size.height),
            ));

            self.pending_tab_drag = Some(pending);
            cx.notify();
            return;
        }

        // Вышли за границы всей полосы — переключаемся с перестановки
        // вкладки на перетаскивание отдельной колонки.
        let viewport = pending.viewport_size;

        let width = (viewport.width * 0.38)
            .min(480.0)
            .clamp(1.0, viewport.width);
        let height = (viewport.height * 0.55)
            .min(520.0)
            .clamp(1.0, viewport.height);

        let column_header_height = self.theme.metrics.column_header_height.max(0.0);

        let origin = Point::new(
            (pointer.x - pending.grab_offset_x).clamp(0.0, (viewport.width - width).max(0.0)),
            (pointer.y - column_header_height - pending.grab_offset_y)
                .clamp(0.0, (viewport.height - height).max(0.0)),
        );

        let bounds = Rect::new(origin, Size::new(width, height));

        // Начало дальнейшего перемещения — точка, в которой
        // вкладка оторвалась от исходной полосы.
        pending.start_position = pointer;

        self.active_tab_drag = Some(ActiveTabDrag {
            pending,
            start_origin: origin,
            bounds,
        });

        // Promote the tab while the mouse is still held. The native
        // floating surface can take over before the user releases the button.
        self.end_tab_drag(cx);
    }

    pub(super) fn end_tab_drag(&mut self, cx: &mut Context<Self>) {
        let had_pending = self.pending_tab_drag.take().is_some();

        let Some(active) = self.active_tab_drag.take() else {
            if had_pending {
                cx.notify();
            }
            return;
        };

        let source = active.pending;

        let panel = self
            .container
            .layout_mut(source.composition_index)
            .and_then(|layout| layout.remove_panel(&source.path, source.panel_index));

        let Some(panel) = panel else {
            cx.notify();
            return;
        };

        self.remove_empty_source_layout(source.composition_index);

        let layout = DockLayout::single_panel(panel);
        let composition_index = self.container.insert_floating_layout(layout);

        self.pending_move = None;
        self.active_move = None;
        self.composition_drop_zone = None;

        self.add_floating_composition(composition_index, active.bounds);

        cx.notify();
    }

    pub(super) fn end_group_drag(&mut self, cx: &mut Context<Self>) {
        let had_pending = self.pending_group_drag.take().is_some();

        let Some(active) = self.active_group_drag.take() else {
            if had_pending {
                cx.notify();
            }
            return;
        };

        let source = active.pending;
        let group = self
            .container
            .layout_mut(source.composition_index)
            .and_then(|layout| layout.take_group(&source.path));

        let Some(group) = group else {
            cx.notify();
            return;
        };

        self.remove_empty_source_layout(source.composition_index);

        let layout = DockLayout::single_group(group);
        let composition_index = self.container.insert_floating_layout(layout);

        self.pending_move = None;
        self.active_move = None;
        self.composition_drop_zone = None;
        self.add_floating_composition(composition_index, active.bounds);

        cx.notify();
    }

    fn remove_empty_source_layout(&mut self, index: usize) {
        let is_empty = self
            .container
            .layout_mut(index)
            .is_some_and(|layout| layout.is_empty());

        if !is_empty || !self.container.remove_empty_layout(index) {
            return;
        }

        self.floating_compositions.retain_mut(|floating| {
            if floating.composition_index == index {
                false
            } else {
                if floating.composition_index > index {
                    floating.composition_index -= 1;
                }
                true
            }
        });
    }

    pub(super) fn begin_composition_move(
        &mut self,
        composition_index: usize,
        position: gpui::Point<gpui::Pixels>,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        self.pending_tab_drag = None;
        self.active_tab_drag = None;
        self.pending_group_drag = None;
        self.active_group_drag = None;
        self.composition_drop_zone = None;
        if self.active_resize.is_some() {
            return;
        }

        if let Some(index) = self
            .floating_compositions
            .iter()
            .position(|floating| floating.composition_index == composition_index)
        {
            let floating = self.floating_compositions.remove(index);
            self.floating_compositions.push(floating);
        }

        self.pending_move = Some(PendingCompositionMove {
            composition_index,
            start_position: Point::new(
                position.x.as_f32(),
                (position.y.as_f32() - geometry::HEADER_HEIGHT).max(0.0),
            ),
            viewport_size: Size::new(viewport_width.max(1.0), viewport_height.max(1.0)),
        });

        self.active_move = None;
    }

    pub(super) fn update_composition_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if self.active_resize.is_some() {
            return;
        }

        if event.pressed_button != Some(MouseButton::Left) {
            self.pending_move = None;
            self.active_move = None;
            return;
        }

        let pointer = Point::new(
            event.position.x.as_f32(),
            (event.position.y.as_f32() - geometry::HEADER_HEIGHT).max(0.0),
        );

        if let Some(active) = self.active_move.clone() {
            let Some(floating) = self
                .floating_compositions
                .iter()
                .find(|floating| floating.composition_index == active.composition_index)
                .copied()
            else {
                self.active_move = None;
                return;
            };

            let bounds = floating.bounds;

            let max_x = (active.viewport_size.width - bounds.size.width).max(0.0);
            let max_y = (active.viewport_size.height - bounds.size.height).max(0.0);

            let origin = Point::new(
                (active.start_origin.x + pointer.x - active.start_position.x).clamp(0.0, max_x),
                (active.start_origin.y + pointer.y - active.start_position.y).clamp(0.0, max_y),
            );

            let moved_bounds = Rect::new(origin, bounds.size);

            if let Some(floating) = self
                .floating_compositions
                .iter_mut()
                .find(|floating| floating.composition_index == active.composition_index)
            {
                floating.bounds = moved_bounds;
            }

            self.composition_drop_zone =
                detect_composition_drop_zone(moved_bounds, active.viewport_size.width);

            cx.notify();
            return;
        }

        let Some(pending) = self.pending_move.clone() else {
            return;
        };

        let dx = pointer.x - pending.start_position.x;
        let dy = pointer.y - pending.start_position.y;

        // A click alone must not detach the composition.
        if dx * dx + dy * dy < 25.0 {
            return;
        }

        let viewport_width = pending.viewport_size.width;
        let viewport_height = pending.viewport_size.height;

        let bounds = self
            .floating_compositions
            .iter()
            .find(|floating| floating.composition_index == pending.composition_index)
            .map(|floating| floating.bounds)
            .unwrap_or_else(|| {
                let width = (viewport_width * 0.82).clamp(1.0, viewport_width);
                let height = (viewport_height * 0.82).clamp(1.0, viewport_height);

                let origin_x = (pending.start_position.x / viewport_width
                    * (viewport_width - width))
                    .clamp(0.0, viewport_width - width);

                Rect::new(Point::new(origin_x, 0.0), Size::new(width, height))
            });

        if let Some(floating) = self
            .floating_compositions
            .iter_mut()
            .find(|floating| floating.composition_index == pending.composition_index)
        {
            floating.bounds = bounds;
        } else {
            self.add_floating_composition(pending.composition_index, bounds);
        }

        self.active_move = Some(ActiveCompositionMove {
            composition_index: pending.composition_index,
            start_position: pending.start_position,
            start_origin: bounds.origin,
            viewport_size: pending.viewport_size,
        });

        self.pending_move = None;

        let max_x = (viewport_width - bounds.size.width).max(0.0);
        let max_y = (viewport_height - bounds.size.height).max(0.0);

        let origin = Point::new(
            (bounds.origin.x + dx).clamp(0.0, max_x),
            (bounds.origin.y + dy).clamp(0.0, max_y),
        );

        let moved_bounds = Rect::new(origin, bounds.size);

        if let Some(floating) = self
            .floating_compositions
            .iter_mut()
            .find(|floating| floating.composition_index == pending.composition_index)
        {
            floating.bounds = moved_bounds;
        }

        self.composition_drop_zone = detect_composition_drop_zone(moved_bounds, viewport_width);

        cx.notify();
    }

    pub(super) fn end_composition_move(&mut self, cx: &mut Context<Self>) {
        let had_pending = self.pending_move.take().is_some();
        let active_move = self.active_move.take();
        let had_active = active_move.is_some();
        let drop_zone = self.composition_drop_zone.take();

        if let (Some(active), Some(drop_zone)) = (active_move, drop_zone) {
            // Dropping at the right edge places the empty slot before the composition.
            let empty_before = matches!(drop_zone, CompositionDropZone::RightEdge);

            if self
                .container
                .reserve_empty_neighbor(active.composition_index, empty_before)
            {
                self.floating_compositions
                    .retain(|floating| floating.composition_index != active.composition_index);
            }
        }

        if had_pending || had_active {
            cx.notify();
        }
    }
}

fn detect_composition_drop_zone(bounds: Rect, viewport_width: f32) -> Option<CompositionDropZone> {
    if bounds.origin.x <= COMPOSITION_DROP_ZONE_WIDTH {
        Some(CompositionDropZone::LeftEdge)
    } else if viewport_width - (bounds.origin.x + bounds.size.width) <= COMPOSITION_DROP_ZONE_WIDTH
    {
        Some(CompositionDropZone::RightEdge)
    } else {
        None
    }
}

impl Render for NextDockWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);

        let tab_drag_preview = self.active_tab_drag.as_ref().map(|active| TabDragPreview {
            panel: active.pending.panel,
            bounds: active.bounds,
        });

        let group_drag_preview = self
            .active_group_drag
            .as_ref()
            .map(|active| GroupDragPreview {
                group: active.pending.group.clone(),
                bounds: active.bounds,
                composition_index: active.pending.composition_index,
                group_key: active.pending.group_key.clone(),
            });

        let tab_reorder_preview = self.pending_tab_drag.as_ref().and_then(|pending| {
            pending.preview_bounds.map(|bounds| TabReorderPreview {
                panel: pending.panel,
                bounds,
                composition_index: pending.composition_index,
                group_key: pending.group_key.clone(),
                panel_index: pending.panel_index,
            })
        });

        #[cfg(windows)]
        let native_floating_ids = {
            self.native_main_handle = NativeWindowHandle::from_window(window);
            self.native_main_viewport = Size::new(width, height);

            if let Some(main_handle) = self.native_main_handle {
                let mut current_ids = self
                    .floating_compositions
                    .iter()
                    .map(|floating| floating.id)
                    .collect::<Vec<_>>();
                current_ids.sort_unstable();

                if self.last_native_sync_ids.as_ref() != Some(&current_ids) {
                    self.last_native_sync_ids = Some(current_ids);
                    let owner = cx.entity().downgrade();
                    window.defer(cx, move |_, app| {
                        super::native_floating::synchronize_floating_windows(
                            owner,
                            main_handle,
                            app,
                        );
                    });
                }
            }

            self.native_floating_windows.keys().copied().collect::<Vec<_>>()
        };
        #[cfg(not(windows))]
        let native_floating_ids: Vec<u64> = Vec::new();

        view::render(
            &self.container,
            width,
            height,
            window,
            &self.theme,
            cx.entity().downgrade(),
            &self.floating_compositions,
            &native_floating_ids,
            self.composition_drop_zone,
            tab_drag_preview,
            group_drag_preview,
            tab_reorder_preview,
        )
    }
}
