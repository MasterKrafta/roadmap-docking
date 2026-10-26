use super::*;

pub(super) struct StackAllocation {
    pub(super) sizes: Vec<f32>,
    pub(super) content_height: f32,
    pub(super) overflows: bool,
}

pub(super) fn allocate_stack_heights(
    layout: &Layout,
    children: &[NodeId],
    ratios: &[f32],
    available: f32,
    axis: Axis,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
) -> Result<StackAllocation, ComputeRectsError> {
    if ratios.len() != children.len() {
        return Err(ComputeRectsError::InvalidLayoutState);
    }
    if ratios.is_empty() {
        return Ok(StackAllocation {
            sizes: Vec::new(),
            content_height: 0.0,
            overflows: false,
        });
    }

    let mut collapsed = Vec::with_capacity(children.len());
    let mut minimums = Vec::with_capacity(children.len());
    for child in children {
        #[cfg(test)]
        visit_probe::record_allocate_stack_child(*child);
        collapsed.push(matches!(layout.node(*child), Some(Node::Tabs(tabs)) if tabs.collapsed));
        // Query the common contract for every child; collapsed nodes report
        // their effective strip height from Layout::minimum_node_size too.
        minimums.push(minimum_node_size(layout, *child, metrics, minimum_sizes)?.height);
    }

    let gap_total = metrics.splitter_thickness * children.len().saturating_sub(1) as f32;
    let required = minimums.iter().sum::<f32>() + gap_total;
    if !required.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }

    // Never shrink a child below its content minimum. When the minimum layout
    // does not fit, return those minimum sizes and let the adapter scroll the
    // resulting content extent.
    if available < required {
        return Ok(StackAllocation {
            sizes: minimums,
            content_height: required,
            overflows: true,
        });
    }

    let normal_weights: Vec<f32> = ratios
        .iter()
        .zip(&collapsed)
        .filter_map(|(ratio, is_collapsed)| if *is_collapsed { None } else { Some(*ratio) })
        .collect();
    let normal_minimums: Vec<f32> = minimums
        .iter()
        .zip(&collapsed)
        .filter_map(|(minimum, is_collapsed)| if *is_collapsed { None } else { Some(*minimum) })
        .collect();
    let fixed_collapsed = minimums
        .iter()
        .zip(&collapsed)
        .filter_map(|(minimum, is_collapsed)| if *is_collapsed { Some(*minimum) } else { None })
        .sum::<f32>();
    let normal_available = available - gap_total - fixed_collapsed;
    let normal_sizes = weighted_sizes(&normal_weights, normal_available, &normal_minimums, axis)?;

    let mut normal_index = 0;
    let sizes = collapsed
        .iter()
        .enumerate()
        .map(|(index, is_collapsed)| {
            if *is_collapsed {
                minimums[index]
            } else {
                let size = normal_sizes[normal_index];
                normal_index += 1;
                size
            }
        })
        .collect();

    Ok(StackAllocation {
        sizes,
        content_height: available,
        overflows: false,
    })
}

pub(super) fn push_vertical_overflow(
    owner: NodeId,
    viewport: Rect,
    content_height: f32,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    if !content_height.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    if content_height <= viewport.size.height {
        return Ok(());
    }
    let content_extent = Rect::new(
        viewport.origin,
        Size::new(viewport.size.width, content_height),
    );
    content_extent
        .validate()
        .map_err(ComputeRectsError::Geometry)?;
    output
        .vertical_overflow
        .push(VerticalOverflow::new(owner, viewport, content_extent));
    Ok(())
}

pub(super) fn weighted_sizes(
    weights: &[f32],
    available: f32,
    minimums: &[f32],
    axis: Axis,
) -> Result<Vec<f32>, ComputeRectsError> {
    if weights.len() != minimums.len() {
        return Err(ComputeRectsError::InvalidLayoutState);
    }
    if weights.is_empty() {
        return Ok(Vec::new());
    }
    if !available.is_finite()
        || minimums
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(ComputeRectsError::Geometry(GeometryError::NonFinite));
    }
    let required = minimums.iter().sum::<f32>();
    if !required.is_finite() {
        return Err(ComputeRectsError::Geometry(
            GeometryError::ArithmeticOverflow,
        ));
    }
    if available < required {
        return Err(ComputeRectsError::InsufficientSpace {
            axis,
            available,
            required,
        });
    }

    // Collapsed children keep their fixed minimum height and are excluded
    // before this function is called. If that leaves only zero-weight expanded
    // children, use equal weights so a valid parent ratio vector still fills the
    // available stack space. Positive weights retain their original proportions.
    let equal_weights;
    let weights = if weights.iter().all(|weight| *weight == 0.0) {
        equal_weights = vec![1.0; weights.len()];
        equal_weights.as_slice()
    } else {
        weights
    };

    let mut output = vec![0.0; weights.len()];
    let mut active: Vec<usize> = (0..weights.len()).collect();
    let mut remaining = available;
    while !active.is_empty() {
        let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
        if !weight_sum.is_finite() || weight_sum <= 0.0 {
            return Err(ComputeRectsError::InvalidLayoutState);
        }
        let undersized = active
            .iter()
            .copied()
            .find(|index| remaining * weights[*index] / weight_sum < minimums[*index]);
        if let Some(index) = undersized {
            output[index] = minimums[index];
            remaining -= minimums[index];
            active.retain(|candidate| *candidate != index);
        } else {
            let last_active_position = active.len().saturating_sub(1);
            let mut assigned = 0.0_f32;
            for (position, index) in active.iter().copied().enumerate() {
                let size = if position == last_active_position {
                    remaining - assigned
                } else {
                    let size = remaining * weights[index] / weight_sum;
                    assigned += size;
                    size
                };
                output[index] = size;
            }
            break;
        }
    }
    Ok(output)
}
