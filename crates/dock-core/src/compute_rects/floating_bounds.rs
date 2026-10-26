use super::*;

fn minimum_floating_size(
    layout: &Layout,
    root: NodeId,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<Size, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_minimum_node_size_helper(root);
    layout
        .minimum_floating_size_with_cache(root, metrics, minimum_sizes)
        .map_err(|error| match error {
            crate::CommandError::InvalidValue { field: "size", reason }
                if reason.contains("overflow") =>
            {
                ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow)
            }
            _ => ComputeRectsError::InvalidLayoutState,
        })
}

#[cfg(test)]
pub(crate) fn clamp_floating_bounds(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    client_area: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    clamp_floating_bounds_with_cache(
        layout,
        root,
        bounds,
        client_area,
        metrics,
        &mut HashMap::new(),
    )
}

pub(super) fn clamp_floating_bounds_with_cache(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    client_area: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<Rect, ComputeRectsError> {
    bounds
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    client_area
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    let minimum = minimum_floating_size(layout, root, metrics, minimum_sizes)?;
    if minimum.width > client_area.size.width {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::X,
            available: client_area.size.width,
            required: minimum.width,
        });
    }
    if minimum.height > client_area.size.height {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::Y,
            available: client_area.size.height,
            required: minimum.height,
        });
    }

    // This helper does not mutate the model. Geometry snapshots may use the
    // returned rectangle transiently; workspace recovery may commit it after
    // reporting the change. Oversized windows are fit only when the subtree's
    // minimum size remains satisfied.
    let width = bounds
        .size
        .width
        .max(minimum.width)
        .min(client_area.size.width);
    let height = bounds
        .size
        .height
        .max(minimum.height)
        .min(client_area.size.height);
    let client_right = client_area
        .checked_right()
        .map_err(ComputeRectsError::Geometry)?;
    let client_bottom = client_area
        .checked_bottom()
        .map_err(ComputeRectsError::Geometry)?;
    if client_right <= client_area.origin.x || client_bottom <= client_area.origin.y {
        return Err(ComputeRectsError::Geometry(
            GeometryError::NonPositiveDimension,
        ));
    }
    // Compute the remaining extent before adding it to the origin. Subtracting
    // a large window size from a rounded far edge can otherwise round the
    // minimum origin one representable f32 step outside the client area.
    let max_x = client_area.origin.x + (client_area.size.width - width);
    let max_y = client_area.origin.y + (client_area.size.height - height);
    if !max_x.is_finite()
        || !max_y.is_finite()
        || max_x < client_area.origin.x
        || max_y < client_area.origin.y
    {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    let origin = Point::new(
        bounds.origin.x.clamp(client_area.origin.x, max_x),
        bounds.origin.y.clamp(client_area.origin.y, max_y),
    );
    let result = Rect::new(origin, Size::new(width, height));
    result
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}
