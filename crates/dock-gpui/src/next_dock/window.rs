//! Native window for the independent dock prototype.

use dock_core::{Point, Rect, Size};
use gpui::{Context, IntoElement, MouseButton, MouseMoveEvent, Render, Window};
use ui::Theme;

use super::geometry;
use super::model::{DockLayout, SplitAxis};
use super::view::{self, SplitResizeSpec};

/// Owns the prototype layout and renders it in a regular GPUI window.
pub(super) struct NextDockWindow {
    theme: Theme,
    layout: DockLayout,
    active_resize: Option<ActiveResize>,
    floating_composition: Option<Rect>,
    pending_move: Option<PendingCompositionMove>,
    active_move: Option<ActiveCompositionMove>,
    composition_drop_zone: Option<CompositionDropZone>,
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
}

#[derive(Clone)]
struct ActiveCompositionMove {
    start_position: Point,
    start_origin: Point,
    viewport_size: Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompositionDropZone {
    LeftEdge,
    RightEdge,
}

const COMPOSITION_DROP_ZONE_WIDTH: f32 = 24.0;

impl NextDockWindow {
    pub(super) fn new(theme: Theme) -> Self {
        Self {
            theme,
            layout: DockLayout::initial(),
            active_resize: None,
            floating_composition: None,
            pending_move: None,
            active_move: None,
            composition_drop_zone: None,
        }
    }
    pub(super) fn begin_resize(
        &mut self,
        spec: SplitResizeSpec,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
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

        let Some(current_weights) = self.layout.split_weights_mut(&spec.path) else {
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

    pub(super) fn begin_composition_move(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        self.composition_drop_zone = None;
        if self.active_resize.is_some() {
            return;
        }

        self.pending_move = Some(PendingCompositionMove {
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
            let Some(bounds) = self.floating_composition else {
                self.active_move = None;
                return;
            };

            let max_x = (active.viewport_size.width - bounds.size.width).max(0.0);
            let max_y = (active.viewport_size.height - bounds.size.height).max(0.0);

            let origin = Point::new(
                (active.start_origin.x + pointer.x - active.start_position.x).clamp(0.0, max_x),
                (active.start_origin.y + pointer.y - active.start_position.y).clamp(0.0, max_y),
            );

            let moved_bounds = Rect::new(origin, bounds.size);

            self.floating_composition = Some(moved_bounds);
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

        let bounds = self.floating_composition.unwrap_or_else(|| {
            let width = (viewport_width * 0.82).clamp(1.0, viewport_width);
            let height = (viewport_height * 0.82).clamp(1.0, viewport_height);

            let origin_x = (pending.start_position.x / viewport_width * (viewport_width - width))
                .clamp(0.0, viewport_width - width);

            Rect::new(Point::new(origin_x, 0.0), Size::new(width, height))
        });

        self.floating_composition = Some(bounds);
        self.active_move = Some(ActiveCompositionMove {
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

        self.floating_composition = Some(moved_bounds);
        self.composition_drop_zone = detect_composition_drop_zone(moved_bounds, viewport_width);

        cx.notify();
    }

    pub(super) fn end_composition_move(&mut self, cx: &mut Context<Self>) {
        let had_pending = self.pending_move.take().is_some();
        let had_active = self.active_move.take().is_some();

        if had_active && self.composition_drop_zone.is_some() {
            // With only one composition, dropping at either window edge
            // restores it to the full dock area.
            self.floating_composition = None;
        }

        self.composition_drop_zone = None;

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

        view::render(
            &self.layout,
            width,
            height,
            window,
            &self.theme,
            cx.entity().downgrade(),
            self.floating_composition,
            self.composition_drop_zone,
        )
    }
}
