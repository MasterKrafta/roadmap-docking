//! Native window for the independent dock prototype.

use gpui::{Context, IntoElement, MouseButton, MouseMoveEvent, Render, Window};
use ui::Theme;

use super::model::{DockLayout, SplitAxis};
use super::view::{self, SplitResizeSpec};

/// Owns the prototype layout and renders it in a regular GPUI window.
pub(super) struct NextDockWindow {
    theme: Theme,
    layout: DockLayout,
    active_resize: Option<ActiveResize>,
}

#[derive(Clone)]
struct ActiveResize {
    spec: SplitResizeSpec,
    start_position: f32,
}

impl NextDockWindow {
    pub(super) fn new(theme: Theme) -> Self {
        Self {
            theme,
            layout: DockLayout::initial(),
            active_resize: None,
        }
    }
    pub(super) fn begin_resize(
        &mut self,
        spec: SplitResizeSpec,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
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
        )
    }
}
