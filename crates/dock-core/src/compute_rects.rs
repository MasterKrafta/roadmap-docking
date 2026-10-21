use crate::{
    Axis, Column, ColumnMode, ColumnRect, ComputeRectsError, DockMetrics, DockSide,
    FloatingGroupRect, FloatingResizeEdgeRect, GeometryError, GroupRect,
    Layout, LayoutRects, Node, NodeId, PanelRect, Point, Rect, ResizeEdge, Size,
    SplitterRect, TabHeaderRect, TabStripRect, Tabs, VerticalOverflow,
};

struct StackAllocation {
    sizes: Vec<f32>,
    content_height: f32,
    overflows: bool,
}

impl Layout {
    /// Resolves the validated model into a snapshot of logical-pixel rectangles.
    ///
    /// Column widths come from the model for Expanded columns and from the
    /// presentation metrics for the two rail modes.
    /// The Canvas receives the remaining horizontal space. If that remainder is
    /// below min_canvas_width, computation fails instead of silently compressing
    /// columns or the Canvas. Vertical content that cannot fit is reported with
    /// a scroll extent. Floating bounds are clamped in this snapshot only; the
    /// persistent Layout and CanvasState are not mutated.
    pub fn compute_rects(
        &self,
        viewport: Rect,
        metrics: &DockMetrics,
    ) -> Result<LayoutRects, ComputeRectsError> {
        metrics.validate().map_err(|_| ComputeRectsError::InvalidMetrics)?;
        viewport
            .validate_non_empty()
            .map_err(ComputeRectsError::Geometry)?;
        self.validate(metrics)
            .map_err(|_| ComputeRectsError::InvalidLayout)?;

        let left_root = self.main_area(DockSide::Left).map(|area| area.root);
        let right_root = self.main_area(DockSide::Right).map(|area| area.root);
        let left_width = match left_root {
            Some(root) => preferred_width(self, root, metrics)?,
            None => 0.0,
        };
        let right_width = match right_root {
            Some(root) => preferred_width(self, root, metrics)?,
            None => 0.0,
        };

        let available_canvas = viewport.size.width - left_width - right_width;
        if !available_canvas.is_finite() {
            return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
        }
        if available_canvas < metrics.min_canvas_width {
            return Err(ComputeRectsError::InsufficientCanvasWidth {
                available: available_canvas,
                minimum: metrics.min_canvas_width,
            });
        }

        let viewport_right = viewport
            .checked_right()
            .map_err(ComputeRectsError::Geometry)?;
        let canvas_x = viewport.origin.x + left_width;
        let right_x = viewport_right - right_width;
        let canvas = Rect::new(
            Point::new(canvas_x, viewport.origin.y),
            Size::new(available_canvas, viewport.size.height),
        );
        canvas.validate().map_err(ComputeRectsError::Geometry)?;

        let mut output = LayoutRects {
            viewport: Some(viewport),
            canvas: Some(canvas),
            ..LayoutRects::default()
        };

        if let Some(root) = left_root {
            let area = Rect::new(
                viewport.origin,
                Size::new(left_width, viewport.size.height),
            );
            layout_node(self, root, area, metrics, &mut output)?;
        }
        if let Some(root) = right_root {
            let area = Rect::new(
                Point::new(right_x, viewport.origin.y),
                Size::new(right_width, viewport.size.height),
            );
            layout_node(self, root, area, metrics, &mut output)?;
        }

        // Preserve deterministic back-to-front order in the geometry snapshot.
        for (id, floating) in self.floating_groups_in_paint_order() {
            let area = self.dock_area(floating.area)
                .ok_or(ComputeRectsError::InvalidLayout)?;
            let bounds = clamp_floating_bounds(
                self,
                area.root,
                floating.bounds,
                viewport,
                metrics,
            )?;
            output.floating_groups.push(FloatingGroupRect::new(
                id,
                bounds,
                floating.z_order,
            ));
            push_resize_edges(id, bounds, metrics, &mut output)?;
            layout_node(self, area.root, bounds, metrics, &mut output)?;
        }

        Ok(output)
    }
}

fn preferred_width(
    layout: &Layout,
    id: NodeId,
    metrics: &DockMetrics,
) -> Result<f32, ComputeRectsError> {
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayout)?;
    let width = match node {
        Node::Column(column) => effective_column_width(column, metrics),
        Node::Tabs(_) | Node::Panel(_) => {
            metrics.default_column_width.max(minimum_node_size(layout, id, metrics)?.width)
        }
        Node::Split(split) if split.axis == Axis::X => {
            let mut width = 0.0;
            for child in &split.children {
                width += preferred_width(layout, *child, metrics)?;
            }
            width += metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            width
        }
        Node::Split(split) => {
            let mut width = 0.0_f32;
            for child in &split.children {
                width = width.max(preferred_width(layout, *child, metrics)?);
            }
            width
        }
    };
    if width.is_finite() && width > 0.0 {
        Ok(width)
    } else {
        Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow))
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
fn minimum_node_size(
    layout: &Layout,
    id: NodeId,
    metrics: &DockMetrics,
) -> Result<Size, ComputeRectsError> {
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayout)?;
    let size = match node {
        Node::Panel(panel) => layout
            .panel_instance(panel.panel_id)
            .ok_or(ComputeRectsError::InvalidLayout)?
            .constraints
            .min_size,
        Node::Tabs(tabs) if tabs.collapsed => Size::new(
            0.0,
            metrics.collapsed_group_height.max(metrics.tab_strip_height),
        ),
        Node::Tabs(tabs) => {
            let mut width = 0.0_f32;
            let mut content_height = 0.0_f32;
            for child in &tabs.children {
                let child_size = minimum_node_size(layout, *child, metrics)?;
                width = width.max(child_size.width);
                content_height = content_height.max(child_size.height);
            }
            let height = metrics
                .min_group_height
                .max(metrics.header_height + metrics.tab_strip_height + content_height);
            Size::new(width, height)
        }
        Node::Split(split) => {
            let mut width = 0.0_f32;
            let mut height = 0.0_f32;
            for child in &split.children {
                let child_size = minimum_node_size(layout, *child, metrics)?;
                match split.axis {
                    Axis::X => {
                        width += child_size.width;
                        height = height.max(child_size.height);
                    }
                    Axis::Y => {
                        width = width.max(child_size.width);
                        height += child_size.height;
                    }
                }
            }
            let gaps = metrics.splitter_thickness
                * split.children.len().saturating_sub(1) as f32;
            match split.axis {
                Axis::X => width += gaps,
                Axis::Y => height += gaps,
            }
            Size::new(width, height)
        }
        Node::Column(column) => {
            let width = effective_column_width(column, metrics);
            if column.mode.is_rail() {
                let slot_count = layout
                    .column_display_order(id)
                    .map_err(|_| ComputeRectsError::InvalidLayout)?
                    .len();
                Size::new(
                    width,
                    metrics.collapsed_group_height.max(metrics.tab_strip_height)
                        * slot_count as f32,
                )
            } else {
                let mut height = 0.0_f32;
                for group in &column.groups {
                    height += minimum_node_size(layout, *group, metrics)?.height;
                }
                height += metrics.splitter_thickness
                    * column.groups.len().saturating_sub(1) as f32;
                Size::new(width, height)
            }
        }
    };

    if !size.width.is_finite() || !size.height.is_finite() || size.width < 0.0 || size.height <= 0.0 {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
    }
    Ok(size)
}

fn layout_node(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayout)?;

    // Columns and vertical splits manage their own stack overflow. Other nodes
    // need an expanded content rect when the parent viewport is shorter than
    // their minimum geometry, with a separate viewport/content extent record.
    let manages_vertical_stack = matches!(node, Node::Column(_))
        || matches!(node, Node::Split(split) if split.axis == Axis::Y);
    let mut content_rect = rect;
    if !manages_vertical_stack {
        let minimum = minimum_node_size(layout, id, metrics)?;
        if content_rect.size.height < minimum.height {
            push_vertical_overflow(id, rect, minimum.height, output)?;
            content_rect.size.height = minimum.height;
            content_rect.validate().map_err(ComputeRectsError::Geometry)?;
        }
    }

    match node {
        Node::Column(column) => layout_column(layout, id, column, content_rect, metrics, output),
        Node::Tabs(tabs) => layout_tabs(layout, id, tabs, content_rect, metrics, output),
        Node::Panel(panel) => {
            let minimum = layout
                .panel_instance(panel.panel_id)
                .ok_or(ComputeRectsError::InvalidLayout)?
                .constraints
                .min_size;
            if content_rect.size.width < minimum.width {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::X,
                    available: content_rect.size.width,
                    required: minimum.width,
                });
            }
            if content_rect.size.height < minimum.height {
                return Err(ComputeRectsError::InsufficientSpace {
                    axis: Axis::Y,
                    available: content_rect.size.height,
                    required: minimum.height,
                });
            }
            output.panels.push(PanelRect::new(panel.panel_id, content_rect));
            Ok(())
        }
        Node::Split(split) => layout_split(layout, id, split, content_rect, metrics, output),
    }
}

fn layout_column(
    layout: &Layout,
    id: NodeId,
    column: &Column,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    output.columns.push(ColumnRect::new(id, rect));
    if column.groups.is_empty() {
        return Ok(());
    }

    // Rail modes flatten tabs into fixed-height panel entries. Their order is
    // canonical: groups top-to-bottom, then tabs inside each group left-to-right.
    // Only geometry changes; the underlying Tabs/Column tree remains untouched.
    if column.mode.is_rail() {
        let slot_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);
        let display_order = layout
            .column_display_order(id)
            .map_err(|_| ComputeRectsError::InvalidLayout)?;
        let required = slot_height * display_order.len() as f32;
        if !required.is_finite() {
            return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
        }
        if rect.size.height < required {
            push_vertical_overflow(id, rect, required, output)?;
        }

        let mut y = rect.origin.y;
        let mut item_index = 0usize;
        while item_index < display_order.len() {
            let group_id = display_order[item_index].group;
            let group_start = y;
            let mut item_count = 0usize;

            while item_index < display_order.len()
                && display_order[item_index].group == group_id
            {
                let item = display_order[item_index];
                let item_rect = Rect::new(
                    Point::new(rect.origin.x, y),
                    Size::new(rect.size.width, slot_height),
                );
                item_rect.validate().map_err(ComputeRectsError::Geometry)?;
                output.tab_headers.push(TabHeaderRect::new(
                    item.group,
                    item.panel,
                    item.tab_index,
                    item_rect,
                ));
                y += slot_height;
                item_count += 1;
                item_index += 1;
            }

            let group_rect = Rect::new(
                Point::new(rect.origin.x, group_start),
                Size::new(rect.size.width, slot_height * item_count as f32),
            );
            group_rect.validate().map_err(ComputeRectsError::Geometry)?;
            output.groups.push(GroupRect::new(group_id, group_rect));
        }
        return Ok(());
    }

    let allocation = allocate_stack_heights(
        layout,
        &column.groups,
        &column.group_ratios,
        rect.size.height,
        Axis::Y,
        metrics,
    )?;
    if allocation.overflows {
        push_vertical_overflow(id, rect, allocation.content_height, output)?;
    }

    let mut y = rect.origin.y;
    for (index, group_id) in column.groups.iter().enumerate() {
        let height = allocation.sizes[index];
        let group_rect = Rect::new(
            Point::new(rect.origin.x, y),
            Size::new(rect.size.width, height),
        );
        layout_node(layout, *group_id, group_rect, metrics, output)?;
        y += height;

        if index + 1 < column.groups.len() {
            push_splitter(
                id,
                index,
                Axis::Y,
                y,
                rect.size.width,
                rect.origin.x,
                metrics,
                output,
            )?;
            y += metrics.splitter_thickness;
        }
    }
    Ok(())
}

fn layout_tabs(
    layout: &Layout,
    id: NodeId,
    tabs: &Tabs,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let collapsed = tabs.collapsed;
    let collapsed_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);
    let minimum_size = minimum_node_size(layout, id, metrics)?;
    let minimum_height = if collapsed {
        collapsed_height
    } else {
        minimum_size.height.max(metrics.min_group_height)
    };
    if rect.size.width < minimum_size.width && !collapsed {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::X,
            available: rect.size.width,
            required: minimum_size.width,
        });
    }
    if rect.size.height < minimum_height {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: Axis::Y,
            available: rect.size.height,
            required: minimum_height,
        });
    }

    // A collapsed Tabs group remains strip-sized even when its parent container
    // (for example a floating window) is taller than the collapsed group.
    let group_rect = if collapsed {
        Rect::new(
            rect.origin,
            Size::new(rect.size.width, collapsed_height),
        )
    } else {
        rect
    };
    output.groups.push(GroupRect::new(id, group_rect));

    let strip_height = if collapsed {
        group_rect.size.height
    } else {
        metrics.tab_strip_height
    };
    let strip_y = if collapsed {
        group_rect.origin.y
    } else {
        group_rect.origin.y + metrics.header_height
    };
    let strip_rect = Rect::new(
        Point::new(group_rect.origin.x, strip_y),
        Size::new(group_rect.size.width, strip_height),
    );
    strip_rect.validate().map_err(ComputeRectsError::Geometry)?;
    output.tab_strips.push(TabStripRect::new(id, strip_rect));

    if !tabs.children.is_empty() {
        let tab_width = group_rect.size.width / tabs.children.len() as f32;
        for (index, child_id) in tabs.children.iter().enumerate() {
            if let Some(Node::Panel(panel)) = layout.node(*child_id) {
                let x = group_rect.origin.x + tab_width * index as f32;
                let width = if index + 1 == tabs.children.len() {
                    (group_rect.origin.x + group_rect.size.width) - x
                } else {
                    tab_width
                };
                let header_rect = Rect::new(
                    Point::new(x, strip_y),
                    Size::new(width, strip_height),
                );
                output.tab_headers.push(TabHeaderRect::new(
                    id,
                    panel.panel_id,
                    index,
                    header_rect,
                ));
            }
        }
    }

    if collapsed {
        return Ok(());
    }

    let content_y = group_rect.origin.y + metrics.header_height + metrics.tab_strip_height;
    let content_height = (group_rect.size.height - metrics.header_height - metrics.tab_strip_height)
        .max(0.0);
    let content_rect = Rect::new(
        Point::new(group_rect.origin.x, content_y),
        Size::new(group_rect.size.width, content_height),
    );
    if let Some(active_child) = tabs.active_child() {
        layout_node(layout, active_child, content_rect, metrics, output)?;
    }
    Ok(())
}

fn layout_split(
    layout: &Layout,
    id: NodeId,
    split: &crate::Split,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    if split.children.is_empty() {
        return Ok(());
    }
    if split.ratios.len() != split.children.len() {
        return Err(ComputeRectsError::InvalidLayout);
    }

    match split.axis {
        Axis::X => {
            let direct_columns = split.children.iter().all(|child| {
                matches!(layout.node(*child), Some(Node::Column(_)))
            });
            let gap_total = metrics.splitter_thickness
                * split.children.len().saturating_sub(1) as f32;
            let content_width = rect.size.width - gap_total;
            let widths = if direct_columns {
                split.children.iter().map(|child| {
                    preferred_width(layout, *child, metrics)
                }).collect::<Result<Vec<_>, _>>()?
            } else {
                let minimum_widths = split.children.iter().map(|child| {
                    let minimum = minimum_node_size(layout, *child, metrics)?.width;
                    Ok(minimum.max(metrics.min_column_width))
                }).collect::<Result<Vec<f32>, ComputeRectsError>>()?;
                weighted_sizes(
                    &split.ratios,
                    content_width,
                    &minimum_widths,
                    Axis::X,
                )?
            };
            let required = widths.iter().sum::<f32>() + gap_total;
            if !required.is_finite() {
                return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
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
                let child_rect = Rect::new(
                    Point::new(x, rect.origin.y),
                    Size::new(widths[index], rect.size.height),
                );
                layout_node(layout, *child, child_rect, metrics, output)?;
                x += widths[index];
                if index + 1 < split.children.len() {
                    push_splitter(
                        id,
                        index,
                        Axis::X,
                        x,
                        rect.size.height,
                        rect.origin.y,
                        metrics,
                        output,
                    )?;
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
                layout_node(layout, *child, child_rect, metrics, output)?;
                y += allocation.sizes[index];
                if index + 1 < split.children.len() {
                    push_splitter(
                        id,
                        index,
                        Axis::Y,
                        y,
                        rect.size.width,
                        rect.origin.x,
                        metrics,
                        output,
                    )?;
                    y += metrics.splitter_thickness;
                }
            }
            Ok(())
        }
    }
}

fn allocate_stack_heights(
    layout: &Layout,
    children: &[NodeId],
    ratios: &[f32],
    available: f32,
    axis: Axis,
    metrics: &DockMetrics,
) -> Result<StackAllocation, ComputeRectsError> {
    if ratios.len() != children.len() {
        return Err(ComputeRectsError::InvalidLayout);
    }
    if ratios.is_empty() {
        return Ok(StackAllocation {
            sizes: Vec::new(),
            content_height: 0.0,
            overflows: false,
        });
    }

    let collapsed_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);
    let mut collapsed = Vec::with_capacity(children.len());
    let mut minimums = Vec::with_capacity(children.len());
    for child in children {
        let is_collapsed = matches!(layout.node(*child), Some(Node::Tabs(tabs)) if tabs.collapsed);
        collapsed.push(is_collapsed);
        if is_collapsed {
            minimums.push(collapsed_height);
        } else {
            minimums.push(minimum_node_size(layout, *child, metrics)?.height);
        }
    }

    let gap_total = metrics.splitter_thickness * children.len().saturating_sub(1) as f32;
    let required = minimums.iter().sum::<f32>() + gap_total;
    if !required.is_finite() {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
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

    let normal_weights: Vec<f32> = ratios.iter().zip(&collapsed)
        .filter_map(|(ratio, is_collapsed)| if *is_collapsed { None } else { Some(*ratio) })
        .collect();
    let normal_minimums: Vec<f32> = minimums.iter().zip(&collapsed)
        .filter_map(|(minimum, is_collapsed)| if *is_collapsed { None } else { Some(*minimum) })
        .collect();
    let fixed_collapsed = minimums.iter().zip(&collapsed)
        .filter_map(|(minimum, is_collapsed)| if *is_collapsed { Some(*minimum) } else { None })
        .sum::<f32>();
    let normal_available = available - gap_total - fixed_collapsed;
    let normal_sizes = weighted_sizes(
        &normal_weights,
        normal_available,
        &normal_minimums,
        axis,
    )?;

    let mut normal_index = 0;
    let sizes = collapsed.iter().map(|is_collapsed| {
        if *is_collapsed {
            collapsed_height
        } else {
            let size = normal_sizes[normal_index];
            normal_index += 1;
            size
        }
    }).collect();

    Ok(StackAllocation {
        sizes,
        content_height: available,
        overflows: false,
    })
}

fn push_vertical_overflow(
    owner: NodeId,
    viewport: Rect,
    content_height: f32,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    if !content_height.is_finite() {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
    }
    if content_height <= viewport.size.height {
        return Ok(());
    }
    let content_extent = Rect::new(
        viewport.origin,
        Size::new(viewport.size.width, content_height),
    );
    content_extent.validate().map_err(ComputeRectsError::Geometry)?;
    output.vertical_overflow.push(VerticalOverflow::new(
        owner,
        viewport,
        content_extent,
    ));
    Ok(())
}

fn clamp_floating_bounds(
    layout: &Layout,
    root: NodeId,
    bounds: Rect,
    client_area: Rect,
    metrics: &DockMetrics,
) -> Result<Rect, ComputeRectsError> {
    bounds.validate_non_empty().map_err(ComputeRectsError::Geometry)?;
    client_area.validate_non_empty().map_err(ComputeRectsError::Geometry)?;
    let minimum = minimum_node_size(layout, root, metrics)?;
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

    // Geometry is clamped only in the returned snapshot. Persistent model bounds
    // are untouched; an oversized stored window is fit to the client area only
    // when doing so still respects its subtree's minimum size.
    let width = bounds.size.width.max(minimum.width).min(client_area.size.width);
    let height = bounds.size.height.max(minimum.height).min(client_area.size.height);
    if width < minimum.width || height < minimum.height {
        return Err(ComputeRectsError::InsufficientSpace {
            axis: if width < minimum.width { Axis::X } else { Axis::Y },
            available: if width < minimum.width { width } else { height },
            required: if width < minimum.width { minimum.width } else { minimum.height },
        });
    }
    let client_right = client_area.checked_right().map_err(ComputeRectsError::Geometry)?;
    let client_bottom = client_area.checked_bottom().map_err(ComputeRectsError::Geometry)?;
    let max_x = client_right - width;
    let max_y = client_bottom - height;
    let origin = Point::new(
        bounds.origin.x.clamp(client_area.origin.x, max_x),
        bounds.origin.y.clamp(client_area.origin.y, max_y),
    );
    let result = Rect::new(origin, Size::new(width, height));
    result.validate_non_empty().map_err(ComputeRectsError::Geometry)?;
    Ok(result)
}

fn weighted_sizes(
    weights: &[f32],
    available: f32,
    minimums: &[f32],
    axis: Axis,
) -> Result<Vec<f32>, ComputeRectsError> {
    if weights.len() != minimums.len() {
        return Err(ComputeRectsError::InvalidLayout);
    }
    if weights.is_empty() {
        return Ok(Vec::new());
    }
    if !available.is_finite()
        || minimums.iter().any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(ComputeRectsError::Geometry(GeometryError::NonFinite));
    }
    let required = minimums.iter().sum::<f32>();
    if !required.is_finite() {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
    }
    if available < required {
        return Err(ComputeRectsError::InsufficientSpace { axis, available, required });
    }

    let mut output = vec![0.0; weights.len()];
    let mut active: Vec<usize> = (0..weights.len()).collect();
    let mut remaining = available;
    while !active.is_empty() {
        let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
        if !weight_sum.is_finite() || weight_sum <= 0.0 {
            return Err(ComputeRectsError::InvalidLayout);
        }
        let undersized = active.iter().copied().find(|index| {
            remaining * weights[*index] / weight_sum < minimums[*index]
        });
        if let Some(index) = undersized {
            output[index] = minimums[index];
            remaining -= minimums[index];
            active.retain(|candidate| *candidate != index);
        } else {
            for index in active {
                output[index] = remaining * weights[index] / weight_sum;
            }
            break;
        }
    }
    Ok(output)
}

fn push_splitter(
    owner: NodeId,
    boundary_index: usize,
    axis: Axis,
    boundary: f32,
    cross_size: f32,
    cross_origin: f32,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let rect = match axis {
        Axis::X => Rect::new(
            Point::new(boundary + (metrics.splitter_thickness - metrics.splitter_hit_zone) / 2.0, cross_origin),
            Size::new(metrics.splitter_hit_zone, cross_size),
        ),
        Axis::Y => Rect::new(
            Point::new(cross_origin, boundary + (metrics.splitter_thickness - metrics.splitter_hit_zone) / 2.0),
            Size::new(cross_size, metrics.splitter_hit_zone),
        ),
    };
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    output.splitters.push(SplitterRect::new(owner, boundary_index, axis, rect));
    Ok(())
}

fn push_resize_edges(
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
        (ResizeEdge::TopLeft, Rect::new(Point::new(left, top), Size::new(width, height))),
        (ResizeEdge::TopRight, Rect::new(Point::new(right, top), Size::new(width, height))),
        (ResizeEdge::BottomLeft, Rect::new(Point::new(left, bottom), Size::new(width, height))),
        (ResizeEdge::BottomRight, Rect::new(Point::new(right, bottom), Size::new(width, height))),
        (ResizeEdge::Left, Rect::new(Point::new(left, top), Size::new(width, rect.size.height))),
        (ResizeEdge::Right, Rect::new(Point::new(right, top), Size::new(width, rect.size.height))),
        (ResizeEdge::Top, Rect::new(Point::new(left, top), Size::new(rect.size.width, height))),
        (ResizeEdge::Bottom, Rect::new(Point::new(left, bottom), Size::new(rect.size.width, height))),
    ];
    // Keep checked edge calculations explicit so arithmetic overflow is rejected.
    if !right_edge.is_finite() || !bottom_edge.is_finite() {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
    }
    for (edge, edge_rect) in edges {
        edge_rect.validate().map_err(ComputeRectsError::Geometry)?;
        output.floating_resize_edges.push(FloatingResizeEdgeRect::new(group, edge, edge_rect));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ComputeRectsError;
    use crate::{
        Axis, ColumnMode, DockMetrics, DockSide, PanelConstraints, PanelKey, Point, Profile,
        Node, Rect, Size,
    };
    use crate::layout_builder::LayoutBuilder;

    const PIXEL_TOLERANCE: f32 = 1.0e-3;

    fn constraints() -> PanelConstraints {
        PanelConstraints::new(
            Size::new(80.0, 60.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        )
    }

    fn key(value: &str) -> PanelKey {
        PanelKey::new(value).unwrap()
    }

    fn viewport(width: f32, height: f32) -> Rect {
        Rect::new(Point::ZERO, Size::new(width, height))
    }

    // Test-only stand-in for the renderer's final logical->physical pixel snap.
    // Production dock-core geometry deliberately does not accept a scale factor.
    fn snap_rect_at_render_boundary(rect: Rect, scale: f32) -> Rect {
        assert!(scale.is_finite() && scale > 0.0);
        let snap = |coordinate: f32| (coordinate * scale).round() / scale;
        let left = snap(rect.origin.x);
        let top = snap(rect.origin.y);
        let right = snap(rect.origin.x + rect.size.width);
        let bottom = snap(rect.origin.y + rect.size.height);
        Rect::new(
            Point::new(left, top),
            Size::new(right - left, bottom - top),
        )
    }

    #[test]
    fn columns_keep_model_width_and_canvas_receives_horizontal_remainder() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);

        let (_, left_panel_node) = builder.panel(key("rect-left"), constraints());
        let left_group = builder.tabs(vec![left_panel_node], 0, false);
        let left_column = builder.column(240.0, ColumnMode::Expanded, vec![left_group], vec![1.0]);

        let (_, inner_panel_node) = builder.panel(key("rect-inner"), constraints());
        let inner_group = builder.tabs(vec![inner_panel_node], 0, false);
        let inner_column = builder.column(300.0, ColumnMode::Expanded, vec![inner_group], vec![1.0]);
        let left_root = builder.split(Axis::X, vec![left_column, inner_column], vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, left_root);

        let (_, right_panel_node) = builder.panel(key("rect-right"), constraints());
        let right_group = builder.tabs(vec![right_panel_node], 0, false);
        let right_column = builder.column(220.0, ColumnMode::Expanded, vec![right_group], vec![1.0]);
        builder.main_area(DockSide::Right, right_column);

        let layout = builder.build();
        let first = layout.compute_rects(viewport(1200.0, 700.0), &metrics).unwrap();
        let second = layout.compute_rects(viewport(1300.0, 500.0), &metrics).unwrap();

        assert_eq!(first.columns[0].rect.size.width, 240.0);
        assert_eq!(first.columns[1].rect.size.width, 300.0);
        assert_eq!(first.columns[2].rect.size.width, 220.0);
        assert_eq!(first.columns[0].rect.origin.x, 0.0);
        assert_eq!(first.columns[1].rect.origin.x, 244.0);
        assert_eq!(first.columns[2].rect.origin.x, 980.0);
        assert_eq!(first.canvas.unwrap().origin.x, 544.0);
        assert_eq!(first.canvas.unwrap().size.width, 436.0);
        assert_eq!(second.columns[0].rect.size.width, 240.0);
        assert_eq!(second.columns[1].rect.size.width, 300.0);
        assert_eq!(second.columns[2].rect.size.width, 220.0);
        assert_eq!(second.canvas.unwrap().size.width, 536.0);
        assert_eq!(first.canvas.unwrap().size.height, 700.0);
        assert_eq!(second.canvas.unwrap().size.height, 500.0);
        assert_eq!(first.splitters.len(), 1);
    }

    #[test]
    fn collapsed_groups_keep_fixed_height_and_are_excluded_from_ratios() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for (index, collapsed) in [(0, true), (1, false), (2, false)] {
            let (_, panel_node) = builder.panel(key(&format!("ratio-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, collapsed));
        }
        let column = builder.column(
            280.0,
            ColumnMode::Expanded,
            groups.clone(),
            vec![0.2, 0.3, 0.5],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();
        let heights: Vec<_> = rects.groups.iter().map(|group| group.rect.size.height).collect();
        assert_eq!(heights[0], metrics.collapsed_group_height);
        assert!((heights[1] / heights[2] - 0.3 / 0.5).abs() < PIXEL_TOLERANCE);
        assert!(
            (heights.iter().sum::<f32>() + 2.0 * metrics.splitter_thickness - 500.0).abs()
                < PIXEL_TOLERANCE
        );
        assert_eq!(rects.panels.len(), 2);
        assert!(rects.panels.iter().all(|panel| panel.rect.size.height > 0.0));
    }

    #[test]
    fn ratio_allocation_clamps_small_group_to_its_panel_minimum_and_redistributes_remainder() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for (index, collapsed) in [(0, true), (1, false), (2, false)] {
            let (_, panel_node) = builder.panel(key(&format!("floor-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, collapsed));
        }
        let column = builder.column(
            280.0,
            ColumnMode::Expanded,
            groups,
            vec![0.8, 0.19, 0.01],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();
        let heights: Vec<_> = rects.groups.iter().map(|group| group.rect.size.height).collect();
        let minimum = metrics.header_height + metrics.tab_strip_height + constraints().min_size.height;

        assert_eq!(heights[0], metrics.collapsed_group_height);
        assert!((heights[2] - minimum).abs() < PIXEL_TOLERANCE);
        assert!((heights[1] - 348.0).abs() < PIXEL_TOLERANCE);
        assert!(heights[2] >= minimum);
        assert!(
            (heights.iter().sum::<f32>() + 2.0 * metrics.splitter_thickness - 500.0).abs()
                < PIXEL_TOLERANCE
        );
    }

    #[test]
    fn active_tab_gets_panel_bounds_and_all_tabs_get_header_rectangles() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (first_panel, first_node) = builder.panel(key("tab-first"), constraints());
        let (active_panel, active_node) = builder.panel(key("tab-active"), constraints());
        let group = builder.tabs(vec![first_node, active_node], 1, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();

        assert_eq!(rects.tab_headers.len(), 2);
        assert_eq!(rects.tab_headers[0].panel, first_panel);
        assert_eq!(rects.tab_headers[1].panel, active_panel);
        assert_eq!(rects.tab_strips[0].group, group);
        assert_eq!(rects.panels.len(), 1);
        assert_eq!(rects.panels[0].panel, active_panel);
        assert_eq!(rects.panels[0].rect.origin.y, metrics.header_height + metrics.tab_strip_height);
        assert!(rects.drop_zones.is_empty());
    }

    #[test]
    fn floating_geometry_preserves_bounds_z_order_and_resize_handles() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("floating-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let model_bounds = Rect::new(Point::new(-30.0, 45.0), Size::new(360.0, 260.0));
        let floating = builder.floating_group(group, model_bounds, 9);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 700.0), &metrics).unwrap();

        let expected_bounds = Rect::new(Point::new(0.0, 45.0), Size::new(360.0, 260.0));
        assert_eq!(rects.floating_groups.len(), 1);
        assert_eq!(rects.floating_groups[0].group, floating);
        assert_eq!(rects.floating_groups[0].rect, expected_bounds);
        assert_eq!(rects.floating_groups[0].z_order, 9);
        assert_eq!(rects.floating_resize_edges.len(), 8);
        assert_eq!(rects.panels[0].panel, panel);
        assert_eq!(rects.panels[0].rect.origin.x, expected_bounds.origin.x);
        assert_eq!(rects.panels[0].rect.origin.y, expected_bounds.origin.y + metrics.header_height + metrics.tab_strip_height);
        // Viewport clamping applies to geometry output, not persistent model bounds.
        assert_eq!(layout.floating_group(floating).unwrap().bounds, model_bounds);
    }

    #[test]
    fn floating_collapsed_group_stays_strip_sized_inside_window_bounds() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("floating-collapsed"), constraints());
        let group = builder.tabs(vec![panel_node], 0, true);
        let bounds = Rect::new(Point::new(20.0, 30.0), Size::new(360.0, 260.0));
        builder.floating_group(group, bounds, 2);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 700.0), &metrics).unwrap();

        assert_eq!(rects.floating_groups[0].rect, bounds);
        assert_eq!(rects.groups[0].rect.size.height, metrics.collapsed_group_height);
        assert_eq!(rects.tab_strips[0].rect.size.height, metrics.collapsed_group_height);
        assert!(rects.panels.is_empty());
        assert!(layout.panel_instance(panel).is_some());
    }

    #[test]
    fn icon_rail_uses_rail_width_and_emits_flattened_panel_targets() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("icon-rail-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(metrics.rail_width, ColumnMode::IconsOnly, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();

        assert_eq!(rects.columns[0].rect.size.width, metrics.rail_width);
        assert_eq!(rects.groups[0].rect.size.height, metrics.collapsed_group_height);
        assert!(rects.tab_strips.is_empty());
        assert_eq!(rects.tab_headers.len(), 1);
        assert_eq!(rects.tab_headers[0].panel, panel);
        assert_eq!(rects.tab_headers[0].index, 0);
        assert_eq!(rects.tab_headers[0].rect.size.height, metrics.collapsed_group_height);
        assert!(rects.panels.is_empty());
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(column))
                if column.mode == ColumnMode::IconsOnly && column.width == metrics.rail_width
        ));
    }

    #[test]
    fn golden_rects_for_single_left_column_are_exact_and_deterministic() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("golden-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let viewport = viewport(1000.0, 600.0);

        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        assert_eq!(layout.compute_rects(viewport, &metrics).unwrap(), rects);

        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(280.0, 0.0), Size::new(720.0, 600.0)))
        );
        assert_eq!(rects.columns.len(), 1);
        assert_eq!(
            rects.columns[0].rect,
            Rect::new(Point::ZERO, Size::new(280.0, 600.0))
        );
        assert_eq!(rects.groups.len(), 1);
        assert_eq!(
            rects.groups[0].rect,
            Rect::new(Point::ZERO, Size::new(280.0, 600.0))
        );
        assert_eq!(rects.tab_strips.len(), 1);
        assert_eq!(
            rects.tab_strips[0].rect,
            Rect::new(Point::new(0.0, 28.0), Size::new(280.0, 28.0))
        );
        assert_eq!(rects.tab_headers.len(), 1);
        assert_eq!(rects.tab_headers[0].panel, panel);
        assert_eq!(rects.tab_headers[0].index, 0);
        assert_eq!(
            rects.tab_headers[0].rect,
            Rect::new(Point::new(0.0, 28.0), Size::new(280.0, 28.0))
        );
        assert_eq!(rects.panels.len(), 1);
        assert_eq!(rects.panels[0].panel, panel);
        assert_eq!(
            rects.panels[0].rect,
            Rect::new(Point::new(0.0, 56.0), Size::new(280.0, 544.0))
        );
        assert!(rects.splitters.is_empty());
        assert!(rects.drop_zones.is_empty());
        assert!(rects.vertical_overflow.is_empty());
        assert!(layout.node(column).is_some());
        assert!(layout.node(group).is_some());
    }

    #[test]
    fn golden_rects_for_1280_by_800_cover_both_docks_columns_groups_and_splitters() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);

        let (_, upper_panel_node) = builder.panel(key("golden-1280-left-upper"), constraints());
        let upper_group = builder.tabs(vec![upper_panel_node], 0, false);
        let (_, lower_panel_node) = builder.panel(key("golden-1280-left-lower"), constraints());
        let lower_group = builder.tabs(vec![lower_panel_node], 0, false);
        let first_column = builder.column(
            240.0,
            ColumnMode::Expanded,
            vec![upper_group, lower_group],
            vec![0.5, 0.5],
        );

        let (_, middle_panel_node) = builder.panel(key("golden-1280-left-middle"), constraints());
        let middle_group = builder.tabs(vec![middle_panel_node], 0, false);
        let second_column = builder.column(
            300.0,
            ColumnMode::Expanded,
            vec![middle_group],
            vec![1.0],
        );
        let left_root = builder.split(
            Axis::X,
            vec![first_column, second_column],
            vec![0.5, 0.5],
        );
        builder.main_area(DockSide::Left, left_root);

        let (_, right_panel_node) = builder.panel(key("golden-1280-right"), constraints());
        let right_group = builder.tabs(vec![right_panel_node], 0, false);
        let right_column = builder.column(
            220.0,
            ColumnMode::Expanded,
            vec![right_group],
            vec![1.0],
        );
        builder.main_area(DockSide::Right, right_column);

        let layout = builder.build();
        let rects = layout.compute_rects(viewport(1280.0, 800.0), &metrics).unwrap();

        // Manual golden values: left dock = 240 + 4 + 300 = 544 px;
        // right dock = 220 px; Canvas gets the exact 516 px remainder.
        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(544.0, 0.0), Size::new(516.0, 800.0)))
        );
        assert_eq!(rects.columns.len(), 3);
        assert_eq!(
            rects.columns.iter().map(|item| item.rect).collect::<Vec<_>>(),
            vec![
                Rect::new(Point::ZERO, Size::new(240.0, 800.0)),
                Rect::new(Point::new(244.0, 0.0), Size::new(300.0, 800.0)),
                Rect::new(Point::new(1060.0, 0.0), Size::new(220.0, 800.0)),
            ]
        );

        assert_eq!(rects.groups.len(), 4);
        assert_eq!(
            rects.groups.iter().map(|item| item.rect).collect::<Vec<_>>(),
            vec![
                Rect::new(Point::ZERO, Size::new(240.0, 398.0)),
                Rect::new(Point::new(0.0, 402.0), Size::new(240.0, 398.0)),
                Rect::new(Point::new(244.0, 0.0), Size::new(300.0, 800.0)),
                Rect::new(Point::new(1060.0, 0.0), Size::new(220.0, 800.0)),
            ]
        );

        // Hit rectangles intentionally use the 8 px splitter hit zone,
        // centered around each 4 px visual splitter.
        assert_eq!(rects.splitters.len(), 2);
        assert_eq!(rects.splitters[0].owner, first_column);
        assert_eq!(rects.splitters[0].boundary_index, 0);
        assert_eq!(rects.splitters[0].axis, Axis::Y);
        assert_eq!(
            rects.splitters[0].rect,
            Rect::new(Point::new(0.0, 396.0), Size::new(240.0, 8.0))
        );
        assert_eq!(rects.splitters[1].owner, left_root);
        assert_eq!(rects.splitters[1].boundary_index, 0);
        assert_eq!(rects.splitters[1].axis, Axis::X);
        assert_eq!(
            rects.splitters[1].rect,
            Rect::new(Point::new(238.0, 0.0), Size::new(8.0, 800.0))
        );

        assert_eq!(rects.panels.len(), 4);
        assert_eq!(
            rects.panels.iter().map(|item| item.rect).collect::<Vec<_>>(),
            vec![
                Rect::new(Point::new(0.0, 56.0), Size::new(240.0, 342.0)),
                Rect::new(Point::new(0.0, 458.0), Size::new(240.0, 342.0)),
                Rect::new(Point::new(244.0, 56.0), Size::new(300.0, 744.0)),
                Rect::new(Point::new(1060.0, 56.0), Size::new(220.0, 744.0)),
            ]
        );
        assert!(rects.vertical_overflow.is_empty());
        assert!(rects.drop_zones.is_empty());
    }

    #[test]
    fn fractional_logical_viewport_preserves_dpi_independent_geometry_and_reports_small_height_overflow() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("fractional-logical-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(
            200.25,
            ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        // Geometry remains in logical px: non-integral viewport origins, widths,
        // and heights are preserved exactly instead of rounded for a display DPI.
        let viewport = Rect::new(
            Point::new(12.25, 7.5),
            Size::new(520.75, 196.5),
        );
        let rects = layout.compute_rects(viewport, &metrics).unwrap();
        assert_eq!(
            rects.canvas,
            Some(Rect::new(
                Point::new(212.5, 7.5),
                Size::new(320.5, 196.5),
            ))
        );
        assert_eq!(
            rects.columns[0].rect,
            Rect::new(
                Point::new(12.25, 7.5),
                Size::new(200.25, 196.5),
            )
        );
        assert_eq!(
            rects.groups[0].rect,
            Rect::new(
                Point::new(12.25, 7.5),
                Size::new(200.25, 196.5),
            )
        );
        assert_eq!(
            rects.panels[0].rect,
            Rect::new(
                Point::new(12.25, 63.5),
                Size::new(200.25, 140.5),
            )
        );
        assert!(rects.vertical_overflow.is_empty());

        // The panel's declared minimum height is 60 px, so the expanded group
        // minimum is header 28 + tab strip 28 + content 60 = 116 logical px.
        // A viewport 0.5 px below that minimum must report overflow.
        let short_viewport = Rect::new(
            Point::new(12.25, 7.5),
            Size::new(520.75, 115.5),
        );
        let short_rects = layout.compute_rects(short_viewport, &metrics).unwrap();
        assert_eq!(short_rects.vertical_overflow.len(), 1);
        assert_eq!(short_rects.vertical_overflow[0].owner, column);
        assert_eq!(
            short_rects.vertical_overflow[0].viewport,
            Rect::new(
                Point::new(12.25, 7.5),
                Size::new(200.25, 115.5),
            )
        );
        assert_eq!(
            short_rects.vertical_overflow[0].content_extent,
            Rect::new(
                Point::new(12.25, 7.5),
                Size::new(200.25, 116.0),
            )
        );
        assert_eq!(short_rects.groups[0].rect.size.height, 116.0);
        assert_eq!(short_rects.panels[0].rect.size.height, 60.0);

        // Crossing the minimum Canvas width by 0.25 logical px must fail,
        // rather than shrinking either dock or the Canvas.
        let too_narrow = Rect::new(
            Point::new(12.25, 7.5),
            Size::new(520.0, 196.5),
        );
        assert!(matches!(
            layout.compute_rects(too_narrow, &metrics),
            Err(ComputeRectsError::InsufficientCanvasWidth { available, minimum })
                if available == 319.75 && minimum == metrics.min_canvas_width
        ));
    }

    #[test]
    fn pixel_snapping_is_deferred_to_the_render_boundary_for_common_dpi_scales() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("dpi-boundary-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(
            200.25,
            ColumnMode::Expanded,
            vec![group],
            vec![1.0],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let viewport = Rect::new(
            Point::new(12.25, 7.5),
            Size::new(520.75, 196.5),
        );
        let expected_logical_panel = Rect::new(
            Point::new(12.25, 63.5),
            Size::new(200.25, 140.5),
        );
        let baseline = layout.compute_rects(viewport, &metrics).unwrap();
        assert_eq!(baseline.panels[0].rect, expected_logical_panel);

        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            // Recomputing core geometry must return identical logical-pixel data.
            let rects = layout.compute_rects(viewport, &metrics).unwrap();
            assert_eq!(rects, baseline, "logical geometry changed at scale {scale}");
            let logical_panel = rects.panels[0].rect;
            assert_eq!(logical_panel, expected_logical_panel);

            // Snapping is a separate operation on a copy at the render boundary.
            let snapped = snap_rect_at_render_boundary(logical_panel, scale);
            assert_ne!(snapped, logical_panel, "fractional edges should snap at scale {scale}");
            let physical_edges = [
                snapped.origin.x * scale,
                snapped.origin.y * scale,
                (snapped.origin.x + snapped.size.width) * scale,
                (snapped.origin.y + snapped.size.height) * scale,
            ];
            for edge in physical_edges {
                assert!(
                    (edge - edge.round()).abs() < PIXEL_TOLERANCE,
                    "edge {edge} is not pixel-aligned at scale {scale}"
                );
            }

            // Applying render snapping cannot mutate the geometry snapshot.
            assert_eq!(rects.panels[0].rect, expected_logical_panel);
        }
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn configurable_metrics_drive_header_strip_and_splitter_geometry() {
        let mut metrics = DockMetrics::default();
        metrics.header_height = 31.5;
        metrics.tab_strip_height = 20.25;
        metrics.splitter_thickness = 3.5;
        metrics.splitter_hit_zone = 9.0;
        assert_eq!(metrics.validate(), Ok(()));

        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for name in ["custom-metrics-upper", "custom-metrics-lower"] {
            let (_, panel_node) = builder.panel(key(name), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, false));
        }
        let column = builder.column(
            280.0,
            ColumnMode::Expanded,
            groups.clone(),
            vec![0.5, 0.5],
        );
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();
        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();

        assert_eq!(
            rects.tab_strips.iter().map(|item| item.rect).collect::<Vec<_>>(),
            vec![
                Rect::new(Point::new(0.0, 31.5), Size::new(280.0, 20.25)),
                Rect::new(Point::new(0.0, 283.25), Size::new(280.0, 20.25)),
            ]
        );
        assert_eq!(
            rects.panels.iter().map(|item| item.rect).collect::<Vec<_>>(),
            vec![
                Rect::new(Point::new(0.0, 51.75), Size::new(280.0, 196.5)),
                Rect::new(Point::new(0.0, 303.5), Size::new(280.0, 196.5)),
            ]
        );
        assert_eq!(rects.splitters.len(), 1);
        assert_eq!(rects.splitters[0].owner, column);
        assert_eq!(
            rects.splitters[0].rect,
            Rect::new(Point::new(0.0, 245.5), Size::new(280.0, 9.0))
        );
        assert_eq!(
            rects.canvas,
            Some(Rect::new(Point::new(280.0, 0.0), Size::new(720.0, 500.0)))
        );

        // The model remains unchanged; these configurable presentation metrics
        // affect only the computed geometry snapshot.
        assert!(layout.node(column).is_some());
        assert_eq!(metrics.header_height, 31.5);
        assert_eq!(metrics.tab_strip_height, 20.25);
        assert_eq!(metrics.splitter_thickness, 3.5);
        assert_eq!(metrics.splitter_hit_zone, 9.0);
    }

    #[test]
    fn floating_bounds_clamp_to_client_area_respect_minimums_and_sort_by_z() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut ids = Vec::new();
        for (name, z_order, origin, size) in [
            ("float-high", 3, Point::new(-100.0, 650.0), Size::new(420.0, 360.0)),
            ("float-low", 1, Point::new(900.0, -50.0), Size::new(420.0, 360.0)),
            ("float-mid", 2, Point::new(30.0, 40.0), Size::new(100.0, 80.0)),
        ] {
            let constraints = if name == "float-mid" {
                PanelConstraints::new(
                    Size::new(160.0, 100.0),
                    Size::new(180.0, 140.0),
                    Some(Size::new(800.0, 600.0)),
                    false,
                )
            } else {
                constraints()
            };
            let (_, panel_node) = builder.panel(key(name), constraints);
            let group = builder.tabs(vec![panel_node], 0, false);
            ids.push(builder.floating_group(
                group,
                Rect::new(origin, size),
                z_order,
            ));
        }
        let layout = builder.build();
        let client = Rect::new(Point::new(10.0, 20.0), Size::new(1000.0, 700.0));

        let rects = layout.compute_rects(client, &metrics).unwrap();

        assert_eq!(
            rects.floating_groups.iter().map(|item| item.z_order).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(rects.floating_groups[0].group, ids[1]);
        assert_eq!(
            rects.floating_groups[0].rect,
            Rect::new(Point::new(590.0, 20.0), Size::new(420.0, 360.0))
        );
        assert_eq!(rects.floating_groups[1].group, ids[2]);
        assert_eq!(
            rects.floating_groups[1].rect,
            Rect::new(Point::new(30.0, 40.0), Size::new(160.0, 156.0))
        );
        assert_eq!(rects.floating_groups[2].group, ids[0]);
        assert_eq!(
            rects.floating_groups[2].rect,
            Rect::new(Point::new(10.0, 360.0), Size::new(420.0, 360.0))
        );
        let small_panel = layout.floating_group(ids[2]).unwrap().bounds;
        assert_eq!(small_panel.size, Size::new(100.0, 80.0));
        assert_eq!(layout.floating_group(ids[0]).unwrap().bounds.origin, Point::new(-100.0, 650.0));
        assert_eq!(rects.floating_resize_edges.len(), 24);
        assert!(rects.vertical_overflow.is_empty());
    }

    #[test]
    fn vertical_split_reports_content_extent_when_child_minimums_do_not_fit() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let mut children = Vec::new();
        for name in ["split-top", "split-bottom"] {
            let (_, panel_node) = builder.panel(key(name), constraints());
            children.push(builder.tabs(vec![panel_node], 0, false));
        }
        let root = builder.split(Axis::Y, children, vec![0.5, 0.5]);
        builder.main_area(DockSide::Left, root);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 120.0), &metrics).unwrap();

        assert_eq!(rects.vertical_overflow.len(), 1);
        assert_eq!(rects.vertical_overflow[0].owner, root);
        assert_eq!(rects.vertical_overflow[0].viewport.size.height, 120.0);
        assert_eq!(rects.vertical_overflow[0].content_extent.size.height, 236.0);
        assert_eq!(rects.panels.len(), 2);
        assert!(rects.panels.iter().all(|panel| panel.rect.size.height == 60.0));
    }

    #[test]
    fn non_finite_viewport_is_rejected_without_emitting_non_finite_rectangles() {
        let layout = LayoutBuilder::new(Profile::Photoshop).build();
        let metrics = DockMetrics::default();
        for viewport in [
            Rect::new(Point::new(f32::NAN, 0.0), Size::new(1000.0, 700.0)),
            Rect::new(Point::ZERO, Size::new(f32::INFINITY, 700.0)),
            Rect::new(Point::ZERO, Size::new(1000.0, f32::INFINITY)),
        ] {
            assert!(matches!(
                layout.compute_rects(viewport, &metrics),
                Err(ComputeRectsError::Geometry(_))
            ));
        }
    }

    #[test]
    fn floating_minimum_must_fit_client_height() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let tight = PanelConstraints::new(
            Size::new(160.0, 100.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let (_, panel_node) = builder.panel(key("float-too-tall"), tight);
        let group = builder.tabs(vec![panel_node], 0, false);
        builder.floating_group(
            group,
            Rect::new(Point::new(30.0, 20.0), Size::new(200.0, 90.0)),
            1,
        );
        let layout = builder.build();

        assert!(matches!(
            layout.compute_rects(viewport(1000.0, 100.0), &metrics),
            Err(ComputeRectsError::InsufficientSpace { axis: Axis::Y, .. })
        ));
    }

    #[test]
    fn refuses_to_shrink_canvas_below_its_minimum_without_overflow_policy() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        for (side, key_text) in [(DockSide::Left, "canvas-left"), (DockSide::Right, "canvas-right")] {
            let (_, panel_node) = builder.panel(key(key_text), constraints());
            let group = builder.tabs(vec![panel_node], 0, false);
            let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
            builder.main_area(side, column);
        }
        let layout = builder.build();

        let result = layout.compute_rects(viewport(800.0, 500.0), &metrics);
        assert!(matches!(
            result,
            Err(ComputeRectsError::InsufficientCanvasWidth {
                available,
                minimum
            }) if available == 240.0 && minimum == metrics.min_canvas_width
        ));
    }

    #[test]
    fn minimum_group_stack_reports_scroll_extent_instead_of_compressing_content() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut groups = Vec::new();
        for index in 0..3 {
            let (_, panel_node) = builder.panel(key(&format!("short-{index}")), constraints());
            groups.push(builder.tabs(vec![panel_node], 0, index == 0));
        }
        let column = builder.column(280.0, ColumnMode::Expanded, groups, vec![0.2, 0.3, 0.5]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 120.0), &metrics).unwrap();
        let overflow = rects.vertical_overflow.as_slice();
        assert_eq!(overflow.len(), 1);
        assert_eq!(overflow[0].owner, column);
        assert_eq!(overflow[0].viewport, viewport(280.0, 120.0));
        assert_eq!(overflow[0].content_extent.origin, Point::ZERO);
        assert_eq!(overflow[0].content_extent.size, Size::new(280.0, 268.0));
        assert_eq!(rects.groups.len(), 3);
        assert_eq!(rects.groups[0].rect.size.height, metrics.collapsed_group_height);
        assert_eq!(rects.groups[1].rect.size.height, 116.0);
        assert_eq!(rects.groups[2].rect.size.height, 116.0);
        assert!(rects.panels.iter().all(|panel| panel.rect.size.height >= constraints().min_size.height));
    }
}
