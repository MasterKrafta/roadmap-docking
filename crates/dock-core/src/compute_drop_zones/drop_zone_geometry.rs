use super::*;

pub(super) fn push_zone(
    zones: &mut Vec<DropZoneRect>,
    target: DropTarget,
    zone: DropZone,
    rect: Rect,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return Ok(());
    }
    zones.push(DropZoneRect::new(target, zone, rect));
    Ok(())
}

pub(super) fn group_edge_rect(
    rect: Rect,
    zone: DropZone,
    edge_width: f32,
) -> Result<Rect, ComputeRectsError> {
    let x_band = edge_width.min(rect.size.width / 2.0);
    let y_band = edge_width.min(rect.size.height / 2.0);
    let side_height = (rect.size.height - 2.0 * y_band).max(0.0);
    let result = match zone {
        DropZone::Left => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + y_band),
            Size::new(x_band, side_height),
        ),
        DropZone::Right => Rect::new(
            Point::new(
                rect.origin.x + rect.size.width - x_band,
                rect.origin.y + y_band,
            ),
            Size::new(x_band, side_height),
        ),
        DropZone::Top => Rect::new(rect.origin, Size::new(rect.size.width, y_band)),
        DropZone::Bottom => Rect::new(
            Point::new(rect.origin.x, rect.origin.y + rect.size.height - y_band),
            Size::new(rect.size.width, y_band),
        ),
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

pub(super) fn group_center_rect(
    rect: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let inset_x = metrics.edge_drop_width.min(rect.size.width / 3.0);
    let content_top = (metrics.header_height + metrics.tab_strip_height).min(rect.size.height);
    let inset_bottom = metrics.edge_drop_width.min(rect.size.height / 3.0);
    let result = Rect::new(
        Point::new(rect.origin.x + inset_x, rect.origin.y + content_top),
        Size::new(
            (rect.size.width - 2.0 * inset_x).max(0.0),
            (rect.size.height - content_top - inset_bottom).max(0.0),
        ),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

pub(super) fn floating_edge_rect_for_branch(
    floating_bounds: Rect,
    column_rect: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let edge_rect = floating_edge_rect(floating_bounds, edge, metrics)?;
    let edge_right = edge_rect
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let column_right = column_rect
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let left = edge_rect.origin.x.max(column_rect.origin.x);
    let right = edge_right.min(column_right);
    let result = Rect::new(
        Point::new(left, edge_rect.origin.y),
        Size::new((right - left).max(0.0), edge_rect.size.height),
    );
    result
        .validate()
        .map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

pub(super) fn floating_edge_rect(
    rect: Rect,
    edge: DropZone,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    let inset_x = metrics.splitter_hit_zone.min(rect.size.width / 3.0);
    let height = metrics.edge_drop_width.min(rect.size.height / 3.0);
    let y = match edge {
        // Keep stack insertion below the floating title and tab-strip hit bands.
        DropZone::Top => rect.origin.y + metrics.header_height + metrics.tab_strip_height,
        DropZone::Bottom => rect.origin.y + rect.size.height - height,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let result = Rect::new(
        Point::new(rect.origin.x + inset_x, y),
        Size::new((rect.size.width - 2.0 * inset_x).max(0.0), height),
    );
    result.validate().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}
