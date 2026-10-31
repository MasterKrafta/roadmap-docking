use std::collections::HashMap;

use dock_core::{FloatingGroupRect, GeometryError, LayoutRects, NodeId, Point, Rect, Size};
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

/// Maps a host-local pointer through every applicable VerticalOverflow viewport.
/// Parent offsets are applied before nested viewport checks, while obscured
/// scroll regions do not get to claim a pointer over a floating composition.
pub(crate) fn map_hit_test_point(
    mut point: Point,
    rects: &LayoutRects,
    scroll_handles: &HashMap<NodeId, ScrollHandle>,
    floating_title_height: f32,
) -> Result<Point, GeometryError> {
    point.validate()?;

    let mut active_floating: Option<FloatingGroupRect> = None;
    for floating in &rects.floating_groups {
        let mut covers_point = floating.rect.contains(point)?;
        if !covers_point {
            for edge in rects
                .floating_resize_edges
                .iter()
                .filter(|edge| edge.group == floating.group)
            {
                if edge.rect.contains(point)? {
                    covers_point = true;
                    break;
                }
            }
        }
        if covers_point && active_floating.is_none_or(|active| floating.z_order > active.z_order) {
            active_floating = Some(*floating);
        }
    }

    if let Some(floating) = active_floating {
        if !floating_title_height.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let title_height = floating_title_height
            .max(0.0)
            .min(floating.rect.size.height);
        let title = Rect::new(
            floating.rect.origin,
            Size::new(floating.rect.size.width, title_height),
        );
        if title.contains(point)? {
            return Ok(point);
        }
        for edge in rects
            .floating_resize_edges
            .iter()
            .filter(|edge| edge.group == floating.group)
        {
            if edge.rect.contains(point)? {
                return Ok(point);
            }
        }
    }

    for overflow in &rects.vertical_overflow {
        if let Some(floating) = active_floating
            && !contains_rect(floating.rect, overflow.viewport)?
        {
            continue;
        }
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

fn contains_rect(outer: Rect, inner: Rect) -> Result<bool, GeometryError> {
    outer.validate()?;
    inner.validate()?;
    let outer_right = outer.checked_right()?;
    let outer_bottom = outer.checked_bottom()?;
    let inner_right = inner.checked_right()?;
    let inner_bottom = inner.checked_bottom()?;
    Ok(inner.origin.x >= outer.origin.x
        && inner.origin.y >= outer.origin.y
        && inner_right <= outer_right
        && inner_bottom <= outer_bottom)
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
