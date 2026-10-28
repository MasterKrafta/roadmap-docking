use std::collections::HashMap;

#[path = "compute_rects/sizing.rs"]
mod sizing;
#[cfg(any(test, feature = "serde"))]
pub(crate) use sizing::clamp_floating_bounds;
use sizing::{clamp_floating_bounds_with_cache, minimum_node_size, preferred_width};

#[path = "compute_rects/stack_allocation.rs"]
mod stack_allocation;
use stack_allocation::{allocate_stack_heights, push_vertical_overflow, weighted_sizes};

#[path = "compute_rects/overlay_geometry.rs"]
mod overlay_geometry;
use overlay_geometry::{append_floating_geometry, compute_flyout_rect, push_resize_edges};

#[path = "compute_rects/node_layout.rs"]
mod node_layout;
use node_layout::layout_node;

#[path = "compute_rects/split_layout.rs"]
mod split_layout;
use split_layout::{layout_floating_root, layout_split, push_splitter};

use crate::{
    Axis, Column, ColumnMode, ColumnRect, ComputeRectsError, DockMetrics, DockSide,
    FloatingGroupRect, FloatingResizeEdgeRect, FlyoutRect, GeometryError, GroupRect, Layout,
    LayoutRects, Node, NodeId, PanelRect, Point, Rect, ResizeEdge, Size, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, Tabs,
};

#[cfg(test)]
mod visit_probe {
    use std::{
        collections::HashMap,
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicBool, Ordering},
        },
    };

    use crate::NodeId;

    #[derive(Clone, Copy, Default)]
    pub(super) struct NodeVisitCounts {
        pub(super) preferred_width: u64,
        pub(super) minimum_node_size_helper: u64,
        pub(super) recursive_minimum_node_size: u64,
        pub(super) allocate_stack_child: u64,
    }

    static ENABLED: AtomicBool = AtomicBool::new(false);
    static COUNTS: OnceLock<Mutex<HashMap<NodeId, NodeVisitCounts>>> = OnceLock::new();

    fn counts() -> &'static Mutex<HashMap<NodeId, NodeVisitCounts>> {
        COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn record(node: NodeId, update: impl FnOnce(&mut NodeVisitCounts)) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut counts) = counts().lock() {
            update(counts.entry(node).or_default());
        }
    }

    pub(super) fn record_preferred_width(node: NodeId) {
        record(node, |counts| counts.preferred_width += 1);
    }

    pub(super) fn record_minimum_node_size_helper(node: NodeId) {
        record(node, |counts| counts.minimum_node_size_helper += 1);
    }

    pub(super) fn record_recursive_minimum_node_size(node: NodeId) {
        record(node, |counts| counts.recursive_minimum_node_size += 1);
    }

    pub(super) fn record_allocate_stack_child(node: NodeId) {
        record(node, |counts| counts.allocate_stack_child += 1);
    }

    pub(super) fn start() {
        ENABLED.store(false, Ordering::Relaxed);
        counts()
            .lock()
            .expect("visit-counter mutex must not be poisoned")
            .clear();
        ENABLED.store(true, Ordering::Relaxed);
    }

    pub(super) fn finish() -> HashMap<NodeId, NodeVisitCounts> {
        ENABLED.store(false, Ordering::Relaxed);
        counts()
            .lock()
            .expect("visit-counter mutex must not be poisoned")
            .clone()
    }
}
#[cfg(test)]
pub(crate) fn record_recursive_minimum_node_size(node: NodeId) {
    visit_probe::record_recursive_minimum_node_size(node);
}

impl Layout {
    /// Resolves the validated model into a snapshot of logical-pixel rectangles.
    ///
    /// Column widths come from the model for Expanded columns and from the
    /// presentation metrics for the two rail modes.
    /// The Canvas receives the remaining horizontal space. If that remainder is
    /// below min_canvas_width, computation fails instead of silently compressing
    /// columns or the Canvas. Vertical content that cannot fit is reported with
    /// a scroll extent. Floating bounds are clamped in this snapshot only; the
    /// persistent Layout and CanvasState are not mutated.
    pub fn compute_rects(
        &self,
        viewport: Rect,
        metrics: &DockMetrics,
    ) -> Result<LayoutRects, ComputeRectsError> {
        metrics
            .validate()
            .map_err(|reason| ComputeRectsError::InvalidMetrics { reason })?;
        viewport
            .validate_non_empty()
            .map_err(ComputeRectsError::Geometry)?;
        self.validate_cached(metrics)
            .map_err(|errors| ComputeRectsError::InvalidLayout { errors })?;

        // Cache minimum sizes only for this immutable geometry pass. The cache
        // cannot outlive the exact layout and metrics used to calculate it.
        let mut minimum_sizes = HashMap::new();

        // Hidden chrome changes presentation only. Keep the main-dock tree in
        // Layout, but omit its geometry so the Canvas gets the full viewport.
        let (left_root, right_root) = if self.chrome().dock_visible() {
            (
                self.main_area(DockSide::Left).map(|area| area.root),
                self.main_area(DockSide::Right).map(|area| area.root),
            )
        } else {
            (None, None)
        };
        let left_width = match left_root {
            Some(root) => preferred_width(self, root, metrics, &mut minimum_sizes)?,
            None => 0.0,
        };
        let right_width = match right_root {
            Some(root) => preferred_width(self, root, metrics, &mut minimum_sizes)?,
            None => 0.0,
        };

        let available_canvas = viewport.size.width - left_width - right_width;
        if !available_canvas.is_finite() {
            return Err(ComputeRectsError::Geometry(
                GeometryError::ArithmeticOverflow,
            ));
        }
        if available_canvas < metrics.min_canvas_width {
            return Err(ComputeRectsError::InsufficientCanvasWidth {
                available: available_canvas,
                minimum: metrics.min_canvas_width,
            });
        }

        let viewport_right = viewport
            .checked_right()
            .map_err(ComputeRectsError::Geometry)?;
        let canvas_x = viewport.origin.x + left_width;
        let right_x = viewport_right - right_width;
        let canvas = Rect::new(
            Point::new(canvas_x, viewport.origin.y),
            Size::new(available_canvas, viewport.size.height),
        );
        canvas.validate().map_err(ComputeRectsError::Geometry)?;

        let mut output = LayoutRects {
            viewport: Some(viewport),
            canvas: Some(canvas),
            ..LayoutRects::default()
        };

        if let Some(root) = left_root {
            let area = Rect::new(viewport.origin, Size::new(left_width, viewport.size.height));
            layout_node(self, root, area, metrics, &mut minimum_sizes, &mut output)?;
        }
        if let Some(root) = right_root {
            let area = Rect::new(
                Point::new(right_x, viewport.origin.y),
                Size::new(right_width, viewport.size.height),
            );
            layout_node(self, root, area, metrics, &mut minimum_sizes, &mut output)?;
        }

        // A rail flyout belongs to the dock presentation. Keep its transient
        // state while hidden, but do not emit geometry until the dock is shown.
        if self.chrome().dock_visible()
            && let Some(flyout) = self.flyout()
        {
            output.flyout = Some(compute_flyout_rect(self, flyout, viewport, &output)?);
        }

        // Floating windows are part of the dock chrome presentation too.
        // Their model bounds and roots stay in Layout while hidden, but neither
        // window geometry nor child-panel geometry is emitted in hidden modes.
        if self.chrome().dock_visible() {
            // Preserve deterministic back-to-front order in the geometry snapshot.
            // A single floating window may not fit the current client area. Build
            // its geometry separately so a local failure cannot discard the main
            // dock or leave partially emitted rectangles for that window.
            for (id, floating) in self.floating_groups_in_paint_order() {
                let area = self
                    .dock_area(floating.area)
                    .ok_or(ComputeRectsError::InvalidLayoutState)?;
                let mut floating_output = LayoutRects::default();
                let result = (|| {
                    let bounds = clamp_floating_bounds_with_cache(
                        self,
                        area.root,
                        floating.bounds,
                        viewport,
                        metrics,
                        &mut minimum_sizes,
                    )?;
                    floating_output.floating_groups.push(FloatingGroupRect::new(
                        id,
                        bounds,
                        floating.z_order,
                    ));
                    push_resize_edges(id, bounds, metrics, &mut floating_output)?;
                    layout_floating_root(
                        self,
                        area.root,
                        bounds,
                        metrics,
                        &mut minimum_sizes,
                        &mut floating_output,
                    )?;
                    Ok::<(), ComputeRectsError>(())
                })();

                match result {
                    Ok(()) => append_floating_geometry(&mut output, floating_output),
                    Err(error) => {
                        tracing::warn!(
                            floating_group = ?id,
                            error = %error,
                            "omitting floating group from geometry snapshot"
                        );
                    }
                }
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
#[path = "compute_rects/tests.rs"]
mod tests;
