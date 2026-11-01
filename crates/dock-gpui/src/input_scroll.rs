use std::collections::HashMap;

use dock_core::{GeometryError, LayoutRects, NodeId, Point};
use gpui::ScrollHandle;

use crate::geometry::point_from_gpui;

/// Converts a pointer in a scrolled viewport to the corresponding unscrolled
/// logical-pixel point expected by Core. Geometry snapshots are never edited.
pub(crate) fn scroll_adjusted_hit_test_point(
    point: Point,
    scroll_offset_y: f32,
) -> Result<Point, GeometryError> {
    if !scroll_offset_y.is_finite() {
        return Err(GeometryError::NonFinite);
    }
    point.checked_offset(0.0, scroll_offset_y.max(0.0))
}

/// Maps a main-host pointer through each applicable VerticalOverflow viewport.
/// Parent offsets are applied before nested viewport checks; geometry snapshots
/// themselves remain immutable.
pub(crate) fn map_hit_test_point(
    mut point: Point,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
) -> Result<Point, GeometryError> {
    point.validate()?;

    for overflow in &rects.vertical_overflow {
        if !overflow.viewport.contains(point)? {
            continue;
        }
        let Some(handle) = scroll_handles.get(&overflow.owner) else {
            continue;
        };
        let offset = point_from_gpui(handle.offset())?;
        point = scroll_adjusted_hit_test_point(point, offset.y)?;
    }

    Ok(point)
}

#[cfg(test)]
mod tests {
    use dock_core::{GeometryError, Point};

    use super::scroll_adjusted_hit_test_point;

    #[test]
    fn scroll_offset_maps_pointer_into_core_content_coordinates() {
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(120.0, 180.0), 240.0),
            Ok(Point::new(120.0, 420.0))
        );
    }

    #[test]
    fn negative_scroll_offset_does_not_move_the_pointer() {
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(10.0, 20.0), -12.0),
            Ok(Point::new(10.0, 20.0))
        );
    }

    #[test]
    fn non_finite_offsets_and_addition_overflow_are_rejected() {
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(10.0, 20.0), f32::NAN),
            Err(GeometryError::NonFinite)
        );
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(10.0, f32::MAX), f32::MAX),
            Err(GeometryError::ArithmeticOverflow)
        );
    }
}
