//! Geometry policy for detaching dock items into floating windows.

use dock_core::{CommandError, DockMetrics, Point, Rect, Size};

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
