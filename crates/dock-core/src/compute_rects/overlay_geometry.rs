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

pub(super) fn compute_flyout_rect(
    layout: &Layout,
    flyout: crate::FlyoutState,
    viewport: Rect,
    output: &LayoutRects,
) -> Result<FlyoutRect, ComputeRectsError> {
    let rail = output
        .columns
        .iter()
        .find(|column| column.column == flyout.column)
        .ok_or(ComputeRectsError::InvalidLayoutState)?;
    let canvas = output.canvas.ok_or(ComputeRectsError::InvalidLayoutState)?;
    let canvas_right = canvas
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let rail_right = rail
        .rect
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;

    // A flyout expands from its rail toward the Canvas and overlays it; it
    // never changes the width allocated to the Canvas itself.
    let (resize_edge, anchor_x, available_width) = if rail_right <= canvas.origin.x {
        (ResizeEdge::Right, rail_right, canvas.size.width)
    } else if rail.rect.origin.x >= canvas_right {
        (ResizeEdge::Left, rail.rect.origin.x, canvas.size.width)
    } else {
        // Flyouts are anchored to main-dock rails, not floating columns.
        return Err(ComputeRectsError::InvalidLayoutState);
    };

    let instance = layout
        .panel_instance(flyout.panel)
        .ok_or(ComputeRectsError::InvalidLayoutState)?;
    let constraints = instance.constraints;
    let min_width = constraints.min_size.width;
    let preferred_width = layout
        .flyout_width(&instance.key)
        .or_else(|| instance.state.last_known_size().map(|size| size.width))
        .unwrap_or(constraints.preferred_size.width);
    let max_width = constraints
        .max_size
        .map(|size| size.width)
        .unwrap_or(f32::MAX)
        .min(available_width);
    if !available_width.is_finite() || max_width < min_width {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::X,
            available: available_width,
            required: min_width,
        });
    }

    let width = preferred_width.clamp(min_width, max_width);
    let x = match resize_edge {
        ResizeEdge::Right => anchor_x,
        ResizeEdge::Left => anchor_x - width,
        _ => return Err(ComputeRectsError::InvalidLayoutState),
    };
    let rect = Rect::new(
        Point::new(x, viewport.origin.y),
        Size::new(width, viewport.size.height),
    );
    rect.validate().map_err(ComputeRectsError::Geometry)?;

    Ok(FlyoutRect::new(
        flyout.column,
        flyout.group,
        flyout.panel,
        rect,
        resize_edge,
    ))
}
