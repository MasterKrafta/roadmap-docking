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
            crate::CommandError::InvalidValue {
                field: "size",
                reason,
            } if reason.contains("overflow") => {
                ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow)
            }
            _ => ComputeRectsError::InvalidLayoutState,
        })
}

#[cfg(any(test, feature = "serde"))]
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


/// Creates the local client rectangle for a native floating window.
/// Core owns the composition's minimum content size, but not its screen position
/// or the viewport of the main application window.
pub(super) fn floating_window_rect_with_cache(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<Rect, ComputeRectsError> {
    bounds
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    let minimum = minimum_floating_size(layout, root, metrics, minimum_sizes)?;
    // Root Tabs geometry reserves the shared CompositionHeader above its Group.
    // Root Columns already account for their own header in their minimum size.
    let composition_header_height = if matches!(layout.node(root), Some(Node::Tabs(_))) {
        metrics.column_header_height
    } else {
        0.0
    };
    let minimum_height = minimum.height + composition_header_height;
    let result = Rect::new(
        Point::ZERO,
        Size::new(
            bounds.size.width.max(minimum.width),
            bounds.size.height.max(minimum_height),
        ),
    );
    result
        .validate_non_empty()
        .map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

#[cfg(any(test, feature = "serde"))]
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

pub(super) fn preferred_width(
    layout: &Layout,
    id: NodeId,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<f32, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_preferred_width(id);
    let node = layout
        .node(id)
        .ok_or(ComputeRectsError::InvalidLayoutState)?;
    let width = match node {
        Node::Column(column) => effective_column_width(column, metrics),
        Node::Tabs(_) | Node::Panel(_) => metrics
            .default_column_width
            .max(minimum_node_size(layout, id, metrics, minimum_sizes)?.width),
        Node::Split(split) if split.axis == Axis::X => {
            let mut width = 0.0;
            for child in &split.children {
                width += preferred_width(layout, *child, metrics, minimum_sizes)?;
            }
            width += metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            width
        }
        Node::Split(split) => {
            let mut width = 0.0_f32;
            for child in &split.children {
                width = width.max(preferred_width(layout, *child, metrics, minimum_sizes)?);
            }
            width
        }
    };
    if width.is_finite() && width > 0.0 {
        Ok(width)
    } else {
        Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ))
    }
}

fn effective_column_width(column: &Column, metrics: &DockMetrics) -> f32 {
    match column.mode {
        ColumnMode::Expanded => column.width,
        ColumnMode::IconsAndLabels => metrics.min_column_width,
        ColumnMode::IconsOnly => metrics.rail_width,
    }
}

/// Computes the minimum geometry of a node, including panel-content constraints
/// and the chrome consumed by expanded Tabs groups.
pub(super) fn minimum_node_size(
    layout: &Layout,
    id: NodeId,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<Size, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_minimum_node_size_helper(id);
    layout
        .minimum_node_size_with_cache(id, metrics, minimum_sizes)
        .map_err(|error| match error {
            crate::CommandError::InvalidValue {
                field: "size",
                reason: "computed minimum size overflowed",
            } => ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow),
            crate::CommandError::InvalidValue { field: "size", .. } => {
                ComputeRectsError::InvalidLayoutState
            }
            _ => ComputeRectsError::InvalidLayoutState,
        })
}
