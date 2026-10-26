use super::*;

pub(super) fn preferred_width(
    layout: &Layout,
    id: NodeId,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<f32, ComputeRectsError> {
    #[cfg(test)]
    visit_probe::record_preferred_width(id);
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayoutState)?;
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
