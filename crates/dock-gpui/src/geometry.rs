use dock_core::{Point, Rect, Size};

/// Translates a rectangle from window coordinates into a container's local coordinates.
pub(crate) fn translate_rect_to_local(rect: Rect, origin: Point) -> Rect {
    Rect::new(
        Point::new(rect.origin.x - origin.x, rect.origin.y - origin.y),
        rect.size,
    )
}

/// Translates a Core drop zone by the GPUI scroll offset and clips it to the viewport.
pub(crate) fn scroll_rect_in_viewport(
    rect: Rect,
    viewport: Rect,
    scroll_offset_y: f32,
) -> Option<Rect> {
    if !scroll_offset_y.is_finite() {
        return None;
    }
    let top = (rect.origin.y + scroll_offset_y).max(viewport.origin.y);
    let bottom = (rect.origin.y + scroll_offset_y + rect.size.height)
        .min(viewport.origin.y + viewport.size.height);
    if !top.is_finite() || !bottom.is_finite() || bottom <= top {
        return None;
    }
    Some(Rect::new(
        Point::new(rect.origin.x, top),
        Size::new(rect.size.width, bottom - top),
    ))
}

/// Returns the smallest rectangle that contains both rectangles.
/// When there is no accumulated rectangle yet, the next rectangle becomes the result.
pub fn union_rects(current: Option<Rect>, next: Rect) -> Rect {
    let Some(current) = current else {
        return next;
    };
    let left = current.origin.x.min(next.origin.x);
    let top = current.origin.y.min(next.origin.y);
    let right = (current.origin.x + current.size.width).max(next.origin.x + next.size.width);
    let bottom = (current.origin.y + current.size.height).max(next.origin.y + next.size.height);
    Rect::new(Point::new(left, top), Size::new(right - left, bottom - top))
}

#[cfg(test)]
mod tests {
    use super::{scroll_rect_in_viewport, translate_rect_to_local, union_rects};
    use dock_core::{Point, Rect, Size};

    #[test]
    fn scrolled_drop_zones_are_translated_and_clipped_to_the_column_viewport() {
        let viewport = Rect::new(
            Point::new(20.0, 10.0),
            Size::new(180.0, 100.0),
        );
        let zone = Rect::new(
            Point::new(20.0, 120.0),
            Size::new(180.0, 40.0),
        );
        assert_eq!(
            scroll_rect_in_viewport(zone, viewport, -30.0),
            Some(Rect::new(
                Point::new(20.0, 90.0),
                Size::new(180.0, 20.0),
            ))
        );
        assert_eq!(scroll_rect_in_viewport(zone, viewport, f32::NAN), None);
    }

    #[test]
    fn floating_descendant_rects_are_relative_to_the_container_origin() {
        let rect = Rect::new(
            Point::new(145.0, 92.0),
            Size::new(180.0, 48.0),
        );
        assert_eq!(
            translate_rect_to_local(rect, Point::new(100.0, 60.0)),
            Rect::new(
                Point::new(45.0, 32.0),
                Size::new(180.0, 48.0),
            ),
        );
    }
    #[test]
    fn union_rects_combines_bounds_and_handles_an_empty_accumulator() {
        let first = Rect::new(
            Point::new(10.0, 20.0),
            Size::new(30.0, 40.0),
        );
        let second = Rect::new(
            Point::new(0.0, 35.0),
            Size::new(20.0, 10.0),
        );

        assert_eq!(union_rects(None, first), first);
        assert_eq!(
            union_rects(Some(first), second),
            Rect::new(
                Point::new(0.0, 20.0),
                Size::new(40.0, 40.0),
            )
        );
    }

}
