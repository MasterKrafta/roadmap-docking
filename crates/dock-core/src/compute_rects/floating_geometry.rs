use super::*;

pub(super) fn append_floating_geometry(output: &mut LayoutRects, floating: LayoutRects) {
    output.columns.extend(floating.columns);
    output.groups.extend(floating.groups);
    output.tab_strips.extend(floating.tab_strips);
    output.tab_headers.extend(floating.tab_headers);
    output.tab_overflows.extend(floating.tab_overflows);
    output.panels.extend(floating.panels);
    output.splitters.extend(floating.splitters);
    output.floating_groups.extend(floating.floating_groups);
    output
        .floating_resize_edges
        .extend(floating.floating_resize_edges);
    output.drop_zones.extend(floating.drop_zones);
    output.vertical_overflow.extend(floating.vertical_overflow);
}

pub(super) fn push_resize_edges(
    group: crate::FloatingGroupId,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let width = metrics.splitter_hit_zone.min(rect.size.width);
    let height = metrics.splitter_hit_zone.min(rect.size.height);
    let left = rect.origin.x;
    let right = rect.origin.x + rect.size.width - width;
    let top = rect.origin.y;
    let bottom = rect.origin.y + rect.size.height - height;
    let right_edge = rect.origin.x + rect.size.width;
    let bottom_edge = rect.origin.y + rect.size.height;
    // Corners precede edges so an adapter can prioritize corner resizing when
    // the hit regions overlap.
    let edges = [
        (
            ResizeEdge::TopLeft,
            Rect::new(Point::new(left, top), Size::new(width, height)),
        ),
        (
            ResizeEdge::TopRight,
            Rect::new(Point::new(right, top), Size::new(width, height)),
        ),
        (
            ResizeEdge::BottomLeft,
            Rect::new(Point::new(left, bottom), Size::new(width, height)),
        ),
        (
            ResizeEdge::BottomRight,
            Rect::new(Point::new(right, bottom), Size::new(width, height)),
        ),
        (
            ResizeEdge::Left,
            Rect::new(Point::new(left, top), Size::new(width, rect.size.height)),
        ),
        (
            ResizeEdge::Right,
            Rect::new(Point::new(right, top), Size::new(width, rect.size.height)),
        ),
        (
            ResizeEdge::Top,
            Rect::new(Point::new(left, top), Size::new(rect.size.width, height)),
        ),
        (
            ResizeEdge::Bottom,
            Rect::new(Point::new(left, bottom), Size::new(rect.size.width, height)),
        ),
    ];
    // Keep checked edge calculations explicit so arithmetic overflow is rejected.
    if !right_edge.is_finite() || !bottom_edge.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    for (edge, edge_rect) in edges {
        edge_rect.validate().map_err(ComputeRectsError::Geometry)?;
        output
            .floating_resize_edges
            .push(FloatingResizeEdgeRect::new(group, edge, edge_rect));
    }
    Ok(())
}
