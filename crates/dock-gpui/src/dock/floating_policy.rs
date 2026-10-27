//! Geometry policy for detaching dock items into floating windows.

use dock_core::{CommandError, DockMetrics, FloatingGroupId, Point, Rect, Size};

/// Builds floating bounds while preserving the pointer's grab offset.
///
/// The source supplies the preferred size. The configured column/group minima are
/// respected when possible, but a narrow viewport always takes precedence.
pub(super) fn initial_float_bounds(
    source_rect: Rect,
    grab_offset: Point,
    pointer: Point,
    viewport: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, CommandError> {
    metrics.validate().map_err(|_| CommandError::InvalidValue {
        field: "metrics",
        reason: "dock metrics are invalid",
    })?;
    source_rect
        .validate_non_empty()
        .map_err(|_| CommandError::InvalidValue {
            field: "source_rect",
            reason: "drag source bounds must be finite and positive",
        })?;
    viewport
        .validate_non_empty()
        .map_err(|_| CommandError::InvalidValue {
            field: "viewport",
            reason: "floating window viewport must be finite and positive",
        })?;
    if !grab_offset.is_finite() || !pointer.is_finite() {
        return Err(CommandError::InvalidValue {
            field: "pointer",
            reason: "drag pointer and grab offset must be finite",
        });
    }

    let width = source_rect
        .size
        .width
        .max(metrics.min_column_width)
        .min(viewport.size.width);
    let minimum_height = metrics
        .min_group_height
        .max(metrics.header_height + metrics.tab_strip_height);
    let height = source_rect
        .size
        .height
        .max(minimum_height)
        .min(viewport.size.height);

    let grab_x = grab_offset.x.clamp(0.0, width);
    let grab_y = grab_offset.y.clamp(0.0, height);
    let desired_origin = Point::new(
        pointer.x - grab_x,
        pointer.y - grab_y - metrics.header_height,
    );
    if !desired_origin.is_finite() {
        return Err(CommandError::InvalidValue {
            field: "origin",
            reason: "floating bounds origin is not finite",
        });
    }

    let viewport_right = viewport.origin.x + viewport.size.width;
    let viewport_bottom = viewport.origin.y + viewport.size.height;
    if !viewport_right.is_finite() || !viewport_bottom.is_finite() {
        return Err(CommandError::InvalidValue {
            field: "viewport",
            reason: "floating window viewport edges are not finite",
        });
    }

    let max_x = viewport_right - width;
    let max_y = viewport_bottom - height;
    let origin = Point::new(
        desired_origin.x.clamp(viewport.origin.x, max_x),
        desired_origin.y.clamp(viewport.origin.y, max_y),
    );
    let bounds = Rect::new(origin, Size::new(width, height));
    bounds
        .validate_non_empty()
        .map_err(|_| CommandError::InvalidValue {
            field: "bounds",
            reason: "computed floating bounds must be finite and positive",
        })?;
    Ok(bounds)
}


/// Keeps a moved floating window inside the same viewport used by Core geometry.
///
/// Core clamps floating rectangles for rendering without rewriting the persistent
/// model. Applying this policy to MoveFloating prevents the saved origin from
/// drifting away from the visible rectangle after a drag.
pub(super) fn clamp_floating_origin(
    requested_origin: Point,
    visible_bounds: Rect,
    viewport: Rect,
) -> Result<Point, CommandError> {
    if !requested_origin.is_finite() {
        return Err(CommandError::InvalidValue {
            field: "origin",
            reason: "floating window origin must be finite",
        });
    }
    visible_bounds
        .validate_non_empty()
        .map_err(|_| CommandError::InvalidValue {
            field: "bounds",
            reason: "visible floating bounds must be finite and positive",
        })?;
    viewport
        .validate_non_empty()
        .map_err(|_| CommandError::InvalidValue {
            field: "viewport",
            reason: "floating window viewport must be finite and positive",
        })?;

    if visible_bounds.size.width > viewport.size.width
        || visible_bounds.size.height > viewport.size.height
    {
        return Err(CommandError::InvalidValue {
            field: "bounds",
            reason: "visible floating window must fit within the viewport",
        });
    }

    let max_x = viewport.origin.x + (viewport.size.width - visible_bounds.size.width);
    let max_y = viewport.origin.y + (viewport.size.height - visible_bounds.size.height);
    if !max_x.is_finite() || !max_y.is_finite() {
        return Err(CommandError::InvalidValue {
            field: "viewport",
            reason: "floating window clamp edges are not finite",
        });
    }

    let origin = Point::new(
        requested_origin.x.clamp(viewport.origin.x, max_x),
        requested_origin.y.clamp(viewport.origin.y, max_y),
    );
    if !origin.is_finite() {
        return Err(CommandError::InvalidValue {
            field: "origin",
            reason: "clamped floating window origin is not finite",
        });
    }
    Ok(origin)
}

/// Finds the frontmost floating window containing the pointer.
///
/// The caller supplies targets in Core's hit-test order, with the frontmost
/// window first. Right and bottom edges are exclusive, matching UI hit regions.
pub(crate) fn floating_group_at_point(
    hit_targets: &[(FloatingGroupId, Rect)],
    pointer: Point,
) -> Option<FloatingGroupId> {
    if !pointer.is_finite() {
        return None;
    }

    hit_targets.iter().find_map(|(group, bounds)| {
        let right = bounds.origin.x + bounds.size.width;
        let bottom = bounds.origin.y + bounds.size.height;
        let contains_pointer = bounds.origin.is_finite()
            && bounds.size.is_finite()
            && bounds.size.width > 0.0
            && bounds.size.height > 0.0
            && right.is_finite()
            && bottom.is_finite()
            && pointer.x >= bounds.origin.x
            && pointer.x < right
            && pointer.y >= bounds.origin.y
            && pointer.y < bottom;
        contains_pointer.then_some(*group)
    })
}

#[cfg(test)]
mod tests {
    use super::{floating_group_at_point, initial_float_bounds};
    use dock_core::{DockMetrics, FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn floating_bounds_preserve_grab_offset_and_account_for_title_bar() {
        let metrics = DockMetrics {
            header_height: 30.0,
            tab_strip_height: 24.0,
            ..DockMetrics::default()
        };
        let source = Rect::new(Point::new(100.0, 80.0), Size::new(400.0, 300.0));
        let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 800.0));

        let bounds = initial_float_bounds(
            source,
            Point::new(80.0, 40.0),
            Point::new(250.0, 180.0),
            viewport,
            &metrics,
        )
        .unwrap();

        close(bounds.origin.x, 170.0);
        close(bounds.origin.y, 110.0);
        close(bounds.size.width, 400.0);
        close(bounds.size.height, 300.0);
    }

    #[test]
    fn floating_bounds_respect_minimum_size_and_clamp_to_viewport_edges() {
        let metrics = DockMetrics {
            default_column_width: 300.0,
            min_column_width: 220.0,
            min_group_height: 180.0,
            ..DockMetrics::default()
        };
        let source = Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 50.0));
        let viewport = Rect::new(Point::new(40.0, 50.0), Size::new(300.0, 240.0));

        let bounds = initial_float_bounds(
            source,
            Point::new(500.0, 500.0),
            Point::new(500.0, 500.0),
            viewport,
            &metrics,
        )
        .unwrap();

        close(bounds.origin.x, 120.0);
        close(bounds.origin.y, 110.0);
        close(bounds.size.width, 220.0);
        close(bounds.size.height, 180.0);
        close(bounds.origin.x + bounds.size.width, 340.0);
        close(bounds.origin.y + bounds.size.height, 290.0);
    }

    #[test]
    fn narrow_viewport_takes_precedence_over_minimum_floating_size() {
        let metrics = DockMetrics::default();
        let source = Rect::new(Point::new(10.0, 20.0), Size::new(500.0, 400.0));
        let viewport = Rect::new(Point::new(30.0, 40.0), Size::new(120.0, 90.0));

        let bounds = initial_float_bounds(
            source,
            Point::new(250.0, 200.0),
            Point::new(600.0, 600.0),
            viewport,
            &metrics,
        )
        .unwrap();

        assert_eq!(bounds, viewport);
    }

    #[test]
    fn invalid_source_viewport_pointer_or_metrics_are_rejected() {
        let metrics = DockMetrics::default();
        let source = Rect::new(Point::ZERO, Size::new(400.0, 300.0));
        let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 800.0));

        assert!(
            initial_float_bounds(
                Rect::new(Point::ZERO, Size::new(0.0, 300.0)),
                Point::ZERO,
                Point::ZERO,
                viewport,
                &metrics,
            )
            .is_err()
        );
        assert!(
            initial_float_bounds(
                source,
                Point::ZERO,
                Point::new(f32::NAN, 0.0),
                viewport,
                &metrics,
            )
            .is_err()
        );
        assert!(
            initial_float_bounds(
                source,
                Point::ZERO,
                Point::ZERO,
                Rect::new(Point::ZERO, Size::new(0.0, 800.0)),
                &metrics,
            )
            .is_err()
        );
        let invalid_metrics = DockMetrics {
            min_canvas_width: f32::NAN,
            ..metrics
        };
        assert!(
            initial_float_bounds(
                source,
                Point::ZERO,
                Point::ZERO,
                viewport,
                &invalid_metrics,
            )
            .is_err()
        );
    }

    #[test]
    fn floating_move_origin_clamps_to_viewport_edges_without_changing_window_size() {
        use super::clamp_floating_origin;

        let viewport = Rect::new(Point::new(10.0, 20.0), Size::new(1000.0, 700.0));
        let visible_bounds = Rect::new(Point::new(40.0, 60.0), Size::new(420.0, 300.0));

        assert_eq!(
            clamp_floating_origin(Point::new(-500.0, -90.0), visible_bounds, viewport).unwrap(),
            Point::new(10.0, 20.0),
            "the window must not move above the client area or beyond its left edge"
        );
        assert_eq!(
            clamp_floating_origin(Point::new(900.0, 900.0), visible_bounds, viewport).unwrap(),
            Point::new(590.0, 420.0),
            "the full rectangle must remain inside the viewport"
        );
        assert_eq!(
            clamp_floating_origin(Point::new(120.0, 140.0), visible_bounds, viewport).unwrap(),
            Point::new(120.0, 140.0),
            "an in-range position must remain unchanged"
        );
    }

    #[test]
    fn floating_hit_test_prefers_the_first_matching_window_in_hit_test_order() {
        let mut groups = SlotMap::<FloatingGroupId, ()>::with_key();
        let back = groups.insert(());
        let front = groups.insert(());
        let back_bounds = Rect::new(Point::new(20.0, 20.0), Size::new(120.0, 100.0));
        let front_bounds = Rect::new(Point::new(60.0, 50.0), Size::new(120.0, 100.0));
        let hit_targets = [(front, front_bounds), (back, back_bounds)];

        assert_eq!(
            floating_group_at_point(&hit_targets, Point::new(80.0, 70.0)),
            Some(front),
            "overlapping windows must choose the frontmost hit target"
        );
        assert_eq!(
            floating_group_at_point(&hit_targets, Point::new(30.0, 30.0)),
            Some(back)
        );
    }

    #[test]
    fn floating_hit_test_uses_half_open_bounds_and_rejects_non_finite_pointers() {
        let mut groups = SlotMap::<FloatingGroupId, ()>::with_key();
        let group = groups.insert(());
        let bounds = Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 80.0));
        let hit_targets = [(group, bounds)];

        assert_eq!(
            floating_group_at_point(&hit_targets, Point::new(10.0, 20.0)),
            Some(group)
        );
        assert_eq!(
            floating_group_at_point(&hit_targets, Point::new(110.0, 60.0)),
            None
        );
        assert_eq!(
            floating_group_at_point(&hit_targets, Point::new(50.0, 100.0)),
            None
        );
        assert_eq!(
            floating_group_at_point(&hit_targets, Point::new(f32::NAN, 20.0)),
            None
        );
    }

}

