#[path = "hit_test/floating.rs"]
mod hit_test_floating;
#[path = "hit_test/geometry.rs"]
mod hit_test_geometry;

use self::hit_test_floating::{all_floating_subtrees, floating_front_to_back, floating_subtree};
use self::hit_test_geometry::{contains, flyout_resize_hit_rect, hit_layout_geometry};
use crate::{
    ComputeRectsError, DockMetrics, HitResult, Layout, LayoutRects, Point, Rect,
};

impl Layout {
    /// Hit-tests a logical-pixel point against a geometry snapshot previously
    /// produced for this same Layout.
    ///
    /// Floating windows form front-to-back layers ordered by descending
    /// z_order. Within a floating window resize grips beat its title band;
    /// then splitters, group headers, tab headers, drop zones and panel content
    /// are tested. Blank space inside the topmost floating window belongs to
    /// that window and does not leak hits to the content beneath it.
    ///
    /// Rectangles use half-open bounds. Invalid coordinates or malformed hit
    /// rectangles are reported instead of allowing NaN/Infinity to affect hit
    /// selection.
    pub fn hit_test(
        &self,
        point: Point,
        rects: &LayoutRects,
        metrics: &DockMetrics,
    ) -> Result<HitResult, ComputeRectsError> {
        metrics
            .validate()
            .map_err(|reason| ComputeRectsError::InvalidMetrics { reason })?;
        point.validate().map_err(ComputeRectsError::Geometry)?;

        let floating_order = floating_front_to_back(&rects.floating_groups);

        // Test front-to-back first. A hit inside a floating window only needs that
        // window's subtree; collecting the union for all windows can wait until the
        // point falls through to the main dock or Canvas.
        for floating in &floating_order {
            if !contains(floating.rect, point)? {
                continue;
            }

            // Resize zones are generated corner-first, so the exact corner hit
            // wins over the overlapping horizontal/vertical edge zones.
            for resize in &rects.floating_resize_edges {
                if resize.group == floating.group && contains(resize.rect, point)? {
                    return Ok(HitResult::FloatingResizeEdge {
                        group: floating.group,
                        edge: resize.edge,
                    });
                }
            }

            let title_height = metrics.header_height.min(floating.rect.size.height);
            let title = Rect::new(
                floating.rect.origin,
                crate::Size::new(floating.rect.size.width, title_height),
            );
            if contains(title, point)? {
                return Ok(HitResult::FloatingTitle {
                    group: floating.group,
                });
            }

            if let Some((nodes, panels)) = floating_subtree(self, floating.group)?
                && let Some(hit) =
                    hit_layout_geometry(self, point, rects, metrics, &nodes, &panels, true)?
            {
                return Ok(hit);
            }

            // A floating window is opaque for hit dispatch purposes: even blank
            // space in a front window must not activate a dock beneath it.
            return Ok(HitResult::FloatingGroup {
                group: floating.group,
            });
        }

        // The flyout overlays the main dock and Canvas, but floating windows
        // above it have already received first refusal in the loop above.
        if let Some(flyout) = rects.flyout {
            let resize_handle = flyout_resize_hit_rect(flyout, metrics)?;
            if contains(resize_handle, point)? {
                return Ok(HitResult::FlyoutResizeEdge {
                    column: flyout.column,
                    panel: flyout.panel,
                    edge: flyout.resize_edge,
                });
            }
            if contains(flyout.rect, point)? {
                return Ok(HitResult::FlyoutPanel {
                    column: flyout.column,
                    group: flyout.group,
                    panel: flyout.panel,
                });
            }
        }

        // Overflow content is larger than its visible viewport. Do not let
        // off-screen main-dock rectangles steal hits outside the client area.
        if let Some(canvas) = rects.canvas {
            let viewport_bottom = canvas
                .checked_bottom()
                .map_err(ComputeRectsError::Geometry)?;
            if point.y < canvas.origin.y || point.y >= viewport_bottom {
                return Ok(HitResult::Miss);
            }
        }

        // Main-dock hit filtering needs to exclude all floating subtrees. Do this
        // only after floating windows, flyout and viewport bounds have declined the point.
        let (floating_nodes, floating_panels) = all_floating_subtrees(self, &floating_order)?;
        if let Some(hit) = hit_layout_geometry(
            self,
            point,
            rects,
            metrics,
            &floating_nodes,
            &floating_panels,
            false,
        )? {
            return Ok(hit);
        }

        if let Some(canvas) = rects.canvas
            && contains(canvas, point)?
        {
            return Ok(HitResult::Canvas);
        }

        // Keep unknown points distinct from the central Canvas.
        Ok(HitResult::Miss)
    }
}

#[cfg(test)]
#[path = "hit_test/tests.rs"]
mod tests;
