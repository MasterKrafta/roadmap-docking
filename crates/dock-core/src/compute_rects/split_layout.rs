use super::*;

pub(super) fn layout_split(
    layout: &Layout,
    id: NodeId,
    split: &crate::Split,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    layout_split_inner(
        SplitLayoutContext { layout, metrics, minimum_sizes, output },
        id,
        split,
        rect,
        false,
    )
}

/// Lays out a floating composition while respecting the independent window title.
///
/// An expanded top group already uses Core's header row for the floating title.
/// A collapsed top group has no such row, so its branch must reserve the same
/// height before allocating group rectangles. Apply this only to composite
/// floating roots; a standalone Tabs root keeps its own established geometry.
pub(super) fn layout_floating_root(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let title_height = metrics.header_height.min(rect.size.height);
    match layout.node(id) {
        Some(Node::Column(column))
            if column.groups.len() > 1
                && layout.floating_branch_starts_collapsed(id)
                && rect.size.height > title_height =>
        {
            let content_rect = Rect::new(
                Point::new(rect.origin.x, rect.origin.y + title_height),
                Size::new(rect.size.width, rect.size.height - title_height),
            );
            layout_node(layout, id, content_rect, metrics, minimum_sizes, output)
        }
        Some(Node::Split(split))
            if split.axis == Axis::Y
                && layout.floating_branch_starts_collapsed(id)
                && rect.size.height > title_height =>
        {
            let content_rect = Rect::new(
                Point::new(rect.origin.x, rect.origin.y + title_height),
                Size::new(rect.size.width, rect.size.height - title_height),
            );
            layout_node(layout, id, content_rect, metrics, minimum_sizes, output)
        }
        Some(Node::Split(split)) if split.axis == Axis::X => {
            layout_split_inner(
                SplitLayoutContext { layout, metrics, minimum_sizes, output },
                id,
                split,
                rect,
                true,
            )
        }
        _ => layout_node(layout, id, rect, metrics, minimum_sizes, output),
    }
}

struct SplitLayoutContext<'a> {
    layout: &'a Layout,
    metrics: &'a DockMetrics,
    minimum_sizes: &'a mut HashMap<NodeId, Size>,
    output: &'a mut LayoutRects,
}

fn layout_split_inner(
    context: SplitLayoutContext<'_>,
    id: NodeId,
    split: &crate::Split,
    rect: Rect,
    reserve_title_for_collapsed_branches: bool,
) -> Result<(), ComputeRectsError> {
    let SplitLayoutContext {
        layout,
        metrics,
        minimum_sizes,
        output,
    } = context;
    if split.children.is_empty() {
        return Ok(());
    }
    if split.ratios.len() != split.children.len() {
        return Err(ComputeRectsError::InvalidLayoutState);
    }

    match split.axis {
        Axis::X => {
            let direct_columns = split
                .children
                .iter()
                .all(|child| matches!(layout.node(*child), Some(Node::Column(_))));
            let gap_total =
                metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            let content_width = rect.size.width - gap_total;
            let widths = if direct_columns {
                split
                    .children
                    .iter()
                    .map(|child| preferred_width(layout, *child, metrics, minimum_sizes))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                let minimum_widths = split
                    .children
                    .iter()
                    .map(|child| {
                        let minimum =
                            minimum_node_size(layout, *child, metrics, minimum_sizes)?.width;
                        Ok(minimum.max(metrics.min_column_width))
                    })
                    .collect::<Result<Vec<f32>, ComputeRectsError>>()?;
                weighted_sizes(&split.ratios, content_width, &minimum_widths, Axis::X)?
            };
            let required = widths.iter().sum::<f32>() + gap_total;
            if !required.is_finite() {
                return Err(ComputeRectsError::Geometry(
                    GeometryError::ArithmeticOverflow,
                ));
            }
            if rect.size.width < required {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::X,
                    available: rect.size.width,
                    required,
                });
            }

            let mut x = rect.origin.x;
            for (index, child) in split.children.iter().enumerate() {
                let mut child_rect = Rect::new(
                    Point::new(x, rect.origin.y),
                    Size::new(widths[index], rect.size.height),
                );
                if reserve_title_for_collapsed_branches
                    && layout.floating_branch_starts_collapsed(*child)
                {
                    let title_height = metrics.header_height.min(child_rect.size.height);
                    if child_rect.size.height > title_height {
                        child_rect.origin.y += title_height;
                        child_rect.size.height -= title_height;
                    }
                }
                layout_node(layout, *child, child_rect, metrics, minimum_sizes, output)?;
                x += widths[index];
                if index + 1 < split.children.len() {
                    push_splitter(id, index, Axis::X, x, rect, metrics, output)?;
                    x += metrics.splitter_thickness;
                }
            }
            Ok(())
        }
        Axis::Y => {
            let allocation = allocate_stack_heights(
                layout,
                &split.children,
                &split.ratios,
                rect.size.height,
                Axis::Y,
                metrics,
                minimum_sizes,
            )?;
            if allocation.overflows {
                push_vertical_overflow(id, rect, allocation.content_height, output)?;
            }
            let mut y = rect.origin.y;
            for (index, child) in split.children.iter().enumerate() {
                let child_rect = Rect::new(
                    Point::new(rect.origin.x, y),
                    Size::new(rect.size.width, allocation.sizes[index]),
                );
                layout_node(layout, *child, child_rect, metrics, minimum_sizes, output)?;
                y += allocation.sizes[index];
                if index + 1 < split.children.len() {
                    push_splitter(id, index, Axis::Y, y, rect, metrics, output)?;
                    y += metrics.splitter_thickness;
                }
            }
            Ok(())
        }
    }
}

pub(super) fn push_splitter(
    owner: NodeId,
    boundary_index: usize,
    axis: Axis,
    boundary: f32,
    parent_rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let (cross_size, cross_origin) = match axis {
        Axis::X => (parent_rect.size.height, parent_rect.origin.y),
        Axis::Y => (parent_rect.size.width, parent_rect.origin.x),
    };
    let rect = match axis {
        Axis::X => Rect::new(
            Point::new(
                boundary + (metrics.splitter_thickness - metrics.splitter_hit_zone) / 2.0,
                cross_origin,
            ),
            Size::new(metrics.splitter_hit_zone, cross_size),
        ),
        Axis::Y => Rect::new(
            Point::new(
                cross_origin,
                boundary + (metrics.splitter_thickness - metrics.splitter_hit_zone) / 2.0,
            ),
            Size::new(cross_size, metrics.splitter_hit_zone),
        ),
    };
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    output
        .splitters
        .push(SplitterRect::new(owner, boundary_index, axis, rect));
    Ok(())
}
