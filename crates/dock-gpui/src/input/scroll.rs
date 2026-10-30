use dock_core::Point;

/// Maps a point in a scrolled viewport back to the unscrolled Core coordinate space.
/// Core rectangles remain immutable while GPUI owns the ephemeral scroll offset.
pub(crate) fn scroll_adjusted_hit_test_point(point: Point, scroll_offset_y: f32) -> Point {
    Point::new(point.x, point.y + scroll_offset_y.max(0.0))
}

#[cfg(test)]
mod tests {
    use dock_core::Point;

    use super::scroll_adjusted_hit_test_point;

    #[test]
    fn scroll_offset_maps_viewport_pointer_back_to_core_content_coordinates() {
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(120.0, 180.0), 240.0),
            Point::new(120.0, 420.0)
        );
    }

    #[test]
    fn negative_scroll_offsets_do_not_move_hit_test_points() {
        assert_eq!(
            scroll_adjusted_hit_test_point(Point::new(10.0, 20.0), -12.0),
            Point::new(10.0, 20.0)
        );
    }
}
