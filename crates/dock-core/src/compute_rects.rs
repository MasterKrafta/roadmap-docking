use crate::{
    Axis, Column, ColumnMode, ColumnRect, ComputeRectsError, DockMetrics, DockSide, DropTarget,
    DropZone, DropZoneRect, FloatingGroupRect, FloatingResizeEdgeRect, GeometryError, GroupRect,
    Layout, LayoutRects, Node, NodeId, PanelRect, Point, Rect, ResizeEdge, Size,
    SplitterRect, TabHeaderRect, TabStripRect, Tabs,
};

impl Layout {
    /// Resolves the validated model into a snapshot of logical-pixel rectangles.
    ///
    /// Column widths come from the model (or rail width for IconsOnly columns).
    /// The Canvas receives the remaining horizontal space. If that remainder is
    /// below min_canvas_width, computation fails instead of silently compressing
    /// columns or the Canvas. This method does not mutate Layout or CanvasState.
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
            output.floating_groups.push(FloatingGroupRect::new(
                id,
                floating.bounds,
                floating.z_order,
            ));
            push_resize_edges(id, floating.bounds, metrics, &mut output)?;
            if let Some(area) = self.dock_area(floating.area) {
                layout_node(self, area.root, floating.bounds, metrics, &mut output)?;
            }
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
        Node::Tabs(_) | Node::Panel(_) => metrics.default_column_width,
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
        ColumnMode::IconsOnly => metrics.rail_width,
    }
}

fn layout_node(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    match layout.node(id).ok_or(ComputeRectsError::InvalidLayout)? {
        Node::Column(column) => layout_column(layout, id, column, rect, metrics, output),
        Node::Tabs(tabs) => layout_tabs(layout, id, tabs, rect, metrics, output),
        Node::Panel(panel) => {
            output.panels.push(PanelRect::new(panel.panel_id, rect));
            Ok(())
        }
        Node::Split(split) => layout_split(layout, id, split, rect, metrics, output),
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

    // An icon rail exposes one fixed hit target per group, not tab strips or
    // panel content. The model's expanded width remains untouched.
    if column.mode == ColumnMode::IconsOnly {
        let slot_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);
        let required = slot_height * column.groups.len() as f32;
        if !required.is_finite() {
            return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
        }
        if rect.size.height < required {
            return Err(ComputeRectsError::InsufficientSpace {
                axis: Axis::Y,
                available: rect.size.height,
                required,
            });
        }
        let mut y = rect.origin.y;
        for group_id in &column.groups {
            output.groups.push(GroupRect::new(
                *group_id,
                Rect::new(
                    Point::new(rect.origin.x, y),
                    Size::new(rect.size.width, slot_height),
                ),
            ));
            y += slot_height;
        }
        return Ok(());
    }

    let collapsed: Vec<bool> = column.groups.iter().map(|group_id| {
        matches!(layout.node(*group_id), Some(Node::Tabs(tabs)) if tabs.collapsed)
    }).collect();
    let heights = allocate_stack_heights(
        &column.group_ratios,
        &collapsed,
        rect.size.height,
        Axis::Y,
        metrics,
    )?;

    let mut y = rect.origin.y;
    for (index, group_id) in column.groups.iter().enumerate() {
        let height = heights[index];
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
    let expanded_minimum = metrics
        .min_group_height
        .max(metrics.header_height + metrics.tab_strip_height);
    let minimum_height = if collapsed { collapsed_height } else { expanded_minimum };
    if !minimum_height.is_finite() {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
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
    push_group_drop_zones(id, group_rect, metrics, output)?;

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
                weighted_sizes(
                    &split.ratios,
                    content_width,
                    metrics.min_column_width,
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
            let collapsed: Vec<bool> = split.children.iter().map(|child_id| {
                matches!(layout.node(*child_id), Some(Node::Tabs(tabs)) if tabs.collapsed)
            }).collect();
            let heights = allocate_stack_heights(
                &split.ratios,
                &collapsed,
                rect.size.height,
                Axis::Y,
                metrics,
            )?;
            let mut y = rect.origin.y;
            for (index, child) in split.children.iter().enumerate() {
                let child_rect = Rect::new(
                    Point::new(rect.origin.x, y),
                    Size::new(rect.size.width, heights[index]),
                );
                layout_node(layout, *child, child_rect, metrics, output)?;
                y += heights[index];
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
    ratios: &[f32],
    collapsed: &[bool],
    available: f32,
    axis: Axis,
    metrics: &DockMetrics,
) -> Result<Vec<f32>, ComputeRectsError> {
    if ratios.len() != collapsed.len() {
        return Err(ComputeRectsError::InvalidLayout);
    }
    if ratios.is_empty() {
        return Ok(Vec::new());
    }

    let collapsed_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);
    let normal_min = metrics
        .min_group_height
        .max(metrics.header_height + metrics.tab_strip_height);
    if !collapsed_height.is_finite() || !normal_min.is_finite() {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
    }

    let gap_total = metrics.splitter_thickness * ratios.len().saturating_sub(1) as f32;
    let fixed_collapsed = collapsed.iter().filter(|value| **value).count() as f32
        * collapsed_height;
    let normal_count = collapsed.iter().filter(|value| !**value).count();
    let required = gap_total + fixed_collapsed + normal_count as f32 * normal_min;
    if !required.is_finite() {
        return Err(ComputeRectsError::Geometry(GeometryError::ArithmeticOverflow));
    }
    if available < required {
        return Err(ComputeRectsError::InsufficientSpace {
            axis,
            available,
            required,
        });
    }

    let normal_available = available - gap_total - fixed_collapsed;
    let mut normal_weights = Vec::with_capacity(normal_count);
    for (index, is_collapsed) in collapsed.iter().enumerate() {
        if !is_collapsed {
            normal_weights.push(ratios[index]);
        }
    }
    let normal_heights = weighted_sizes(&normal_weights, normal_available, normal_min, axis)?;

    let mut normal_index = 0;
    Ok(collapsed.iter().map(|is_collapsed| {
        if *is_collapsed {
            collapsed_height
        } else {
            let height = normal_heights[normal_index];
            normal_index += 1;
            height
        }
    }).collect())
}

fn weighted_sizes(
    weights: &[f32],
    available: f32,
    minimum: f32,
    axis: Axis,
) -> Result<Vec<f32>, ComputeRectsError> {
    if weights.is_empty() {
        return Ok(Vec::new());
    }
    if !available.is_finite() || !minimum.is_finite() || minimum <= 0.0 {
        return Err(ComputeRectsError::Geometry(GeometryError::NonFinite));
    }
    let required = minimum * weights.len() as f32;
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
            remaining * weights[*index] / weight_sum < minimum
        });
        if let Some(index) = undersized {
            output[index] = minimum;
            remaining -= minimum;
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

fn push_group_drop_zones(
    group: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let x_band = metrics.edge_drop_width.min(rect.size.width);
    let y_band = metrics.edge_drop_width.min(rect.size.height);
    let zones = [
        (DropZone::Left, Rect::new(rect.origin, Size::new(x_band, rect.size.height))),
        (
            DropZone::Right,
            Rect::new(
                Point::new(rect.origin.x + rect.size.width - x_band, rect.origin.y),
                Size::new(x_band, rect.size.height),
            ),
        ),
        (DropZone::Top, Rect::new(rect.origin, Size::new(rect.size.width, y_band))),
        (
            DropZone::Bottom,
            Rect::new(
                Point::new(rect.origin.x, rect.origin.y + rect.size.height - y_band),
                Size::new(rect.size.width, y_band),
            ),
        ),
    ];
    for (zone, zone_rect) in zones {
        zone_rect.validate().map_err(ComputeRectsError::Geometry)?;
        output.drop_zones.push(DropZoneRect {
            target: DropTarget::GroupEdge { group, zone },
            zone,
            rect: zone_rect,
        });
    }
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
        assert_eq!(heights.iter().sum::<f32>() + 2.0 * metrics.splitter_thickness, 500.0);
        assert_eq!(rects.panels.len(), 2);
        assert!(rects.panels.iter().all(|panel| panel.rect.size.height > 0.0));
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
        assert_eq!(rects.drop_zones.len(), 4);
    }

    #[test]
    fn floating_geometry_preserves_bounds_z_order_and_resize_handles() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (panel, panel_node) = builder.panel(key("floating-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let bounds = Rect::new(Point::new(-30.0, 45.0), Size::new(360.0, 260.0));
        let floating = builder.floating_group(group, bounds, 9);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 700.0), &metrics).unwrap();

        assert_eq!(rects.floating_groups.len(), 1);
        assert_eq!(rects.floating_groups[0].group, floating);
        assert_eq!(rects.floating_groups[0].rect, bounds);
        assert_eq!(rects.floating_groups[0].z_order, 9);
        assert_eq!(rects.floating_resize_edges.len(), 8);
        assert_eq!(rects.panels[0].panel, panel);
        assert_eq!(rects.panels[0].rect.origin.x, bounds.origin.x);
        assert_eq!(rects.panels[0].rect.origin.y, bounds.origin.y + metrics.header_height + metrics.tab_strip_height);
    }

    #[test]
    fn icon_rail_uses_rail_width_without_mutating_logical_column_width() {
        let metrics = DockMetrics::default();
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel_node) = builder.panel(key("icon-rail-panel"), constraints());
        let group = builder.tabs(vec![panel_node], 0, false);
        let column = builder.column(280.0, ColumnMode::IconsOnly, vec![group], vec![1.0]);
        builder.main_area(DockSide::Left, column);
        let layout = builder.build();

        let rects = layout.compute_rects(viewport(1000.0, 500.0), &metrics).unwrap();

        assert_eq!(rects.columns[0].rect.size.width, metrics.rail_width);
        assert_eq!(rects.groups[0].rect.size.height, metrics.collapsed_group_height);
        assert!(rects.tab_strips.is_empty());
        assert!(rects.tab_headers.is_empty());
        assert!(rects.panels.is_empty());
        assert!(matches!(
            layout.node(column),
            Some(Node::Column(column)) if column.width == 280.0
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
    fn refuses_group_stack_smaller_than_minimum_heights() {
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

        assert!(matches!(
            layout.compute_rects(viewport(1000.0, 120.0), &metrics),
            Err(ComputeRectsError::InsufficientSpace { axis: Axis::Y, .. })
        ));
    }
}
