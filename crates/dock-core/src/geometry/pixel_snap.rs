//! Logical-to-physical pixel alignment helpers for paint adapters.
//!
//! Core layout and hit-testing keep their original logical coordinates. The UI/paint
//! adapter may snap the edges of a rectangle immediately before painting it so that
//! splitter borders land on physical-pixel boundaries at fractional display scales.

use super::{Point, Rect, Size};

/// Snap one logical coordinate to the nearest physical-pixel boundary.
///
/// `scale_factor` is the number of physical pixels per logical pixel (for example,
/// 1.25 on a 125% display). Invalid inputs are returned unchanged.
pub fn snap_logical_to_physical_pixel(value: f32, scale_factor: f32) -> f32 {
    if !value.is_finite() || !scale_factor.is_finite() || scale_factor <= 0.0 {
        return value;
    }

    let scale = f64::from(scale_factor);
    let snapped = (f64::from(value) * scale).round() / scale;
    let snapped = snapped as f32;
    if snapped.is_finite() {
        snapped
    } else {
        value
    }
}

/// Snap every edge of a logical rectangle to the physical-pixel grid.
///
/// The rectangle's right and bottom edges are snapped independently from its origin;
/// its size is then reconstructed from those edges. Adjacent rectangles sharing an
/// edge therefore resolve to the same physical coordinate. Invalid rectangles or
/// scale factors are returned unchanged.
///
/// Use this at the rendering boundary only. Keep `LayoutRects` unsnapped for
/// geometry calculations and hit testing.
pub fn snap_rect_to_physical_pixels(rect: Rect, scale_factor: f32) -> Rect {
    if !scale_factor.is_finite()
        || scale_factor <= 0.0
        || !rect.origin.x.is_finite()
        || !rect.origin.y.is_finite()
        || !rect.size.width.is_finite()
        || !rect.size.height.is_finite()
        || rect.size.width < 0.0
        || rect.size.height < 0.0
    {
        return rect;
    }

    let right = rect.origin.x + rect.size.width;
    let bottom = rect.origin.y + rect.size.height;
    if !right.is_finite() || !bottom.is_finite() {
        return rect;
    }

    let left = snap_logical_to_physical_pixel(rect.origin.x, scale_factor);
    let top = snap_logical_to_physical_pixel(rect.origin.y, scale_factor);
    let right = snap_logical_to_physical_pixel(right, scale_factor);
    let bottom = snap_logical_to_physical_pixel(bottom, scale_factor);

    Rect::new(
        Point::new(left, top),
        Size::new((right - left).max(0.0), (bottom - top).max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::{snap_logical_to_physical_pixel, snap_rect_to_physical_pixels};
    use crate::{Point, Rect, Size};

    const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

    fn assert_pixel_aligned(logical: f32, scale_factor: f32) {
        let physical = logical * scale_factor;
        assert!(
            (physical - physical.round()).abs() < 1.0e-3,
            "{logical} logical px at scale {scale_factor} maps to {physical} physical px"
        );
    }

    #[test]
    fn logical_coordinates_snap_at_supported_display_scales() {
        for scale in SCALES {
            let snapped = snap_logical_to_physical_pixel(13.3, scale);
            assert_pixel_aligned(snapped, scale);
            assert!(
                (snapped - 13.3).abs() <= 0.5 / scale + 1.0e-4,
                "snapping at scale {scale} moved the coordinate too far"
            );
        }
    }

    #[test]
    fn splitter_rectangle_edges_align_to_physical_pixels() {
        // Four logical pixels is the default splitter thickness. Its paint rectangle
        // must have crisp edges even when the logical origin is fractional.
        let splitter = Rect::new(Point::new(101.3, 27.7), Size::new(4.0, 280.3));

        for scale in SCALES {
            let snapped = snap_rect_to_physical_pixels(splitter, scale);
            assert_pixel_aligned(snapped.origin.x, scale);
            assert_pixel_aligned(snapped.origin.y, scale);
            assert_pixel_aligned(snapped.origin.x + snapped.size.width, scale);
            assert_pixel_aligned(snapped.origin.y + snapped.size.height, scale);
            assert!(snapped.size.width >= 0.0 && snapped.size.height >= 0.0);
        }
    }

    #[test]
    fn adjacent_rectangles_keep_a_shared_pixel_aligned_boundary() {
        for scale in SCALES {
            let left = snap_rect_to_physical_pixels(
                Rect::new(Point::new(10.13, 0.0), Size::new(20.17, 10.0)),
                scale,
            );
            let right = snap_rect_to_physical_pixels(
                Rect::new(Point::new(30.3, 0.0), Size::new(15.0, 10.0)),
                scale,
            );
            let left_edge = left.origin.x + left.size.width;
            assert!(
                (left_edge - right.origin.x).abs() < 1.0e-3,
                "adjacent edges diverged at scale {scale}: {left_edge} vs {}",
                right.origin.x
            );
            assert_pixel_aligned(left_edge, scale);
        }
    }

    #[test]
    fn invalid_scale_factors_leave_coordinates_and_rectangles_unchanged() {
        let rect = Rect::new(Point::new(10.3, 20.7), Size::new(4.0, 80.0));
        for invalid_scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(snap_logical_to_physical_pixel(13.3, invalid_scale), 13.3);
            assert_eq!(snap_rect_to_physical_pixels(rect, invalid_scale), rect);
        }
    }
}
