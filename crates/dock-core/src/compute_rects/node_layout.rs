use super::*;


pub(super) fn layout_node(
    layout: &Layout,
    id: NodeId,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    rect.validate().map_err(ComputeRectsError::Geometry)?;
    let node = layout.node(id).ok_or(ComputeRectsError::InvalidLayoutState)?;

    // Columns and vertical splits manage their own stack overflow. Other nodes
    // need an expanded content rect when the parent viewport is shorter than
    // their minimum geometry, with a separate viewport/content extent record.
    let manages_vertical_stack = matches!(node, Node::Column(_))
        || matches!(node, Node::Split(split) if split.axis == Axis::Y);
    let mut content_rect = rect;
    if !manages_vertical_stack {
        let minimum = minimum_node_size(layout, id, metrics, minimum_sizes)?;
        if content_rect.size.height < minimum.height {
            push_vertical_overflow(id, rect, minimum.height, output)?;
            content_rect.size.height = minimum.height;
            content_rect
                .validate()
                .map_err(ComputeRectsError::Geometry)?;
        }
    }

    match node {
        Node::Column(column) => layout_column(
            layout,
            id,
            column,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
        Node::Tabs(tabs) => layout_tabs(
            layout,
            id,
            tabs,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
        Node::Panel(panel) => {
            let minimum = layout
                .panel_instance(panel.panel_id)
                .ok_or(ComputeRectsError::InvalidLayoutState)?
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
            output
                .panels
                .push(PanelRect::new(panel.panel_id, content_rect));
            Ok(())
        }
        Node::Split(split) => layout_split(
            layout,
            id,
            split,
            content_rect,
            metrics,
            minimum_sizes,
            output,
        ),
    }
}

pub(super) fn layout_column(
    layout: &Layout,
    id: NodeId,
    column: &Column,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
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
        let slot_height = metrics.effective_collapsed_group_height();
        let display_order = layout
            .column_display_order(id)
            .map_err(|_| ComputeRectsError::InvalidLayoutState)?;
        let required = slot_height * display_order.len() as f32;
        if !required.is_finite() {
            return Err(ComputeRectsError::Geometry(
                GeometryError::ArithmeticOverflow,
            ));
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

            while item_index < display_order.len() && display_order[item_index].group == group_id {
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
        minimum_sizes,
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
        layout_node(layout, *group_id, group_rect, metrics, minimum_sizes, output)?;
        y += height;

        if index + 1 < column.groups.len() {
            push_splitter(id, index, Axis::Y, y, rect, metrics, output)?;
            y += metrics.splitter_thickness;
        }
    }
    Ok(())
}

pub(super) fn layout_tabs(
    layout: &Layout,
    id: NodeId,
    tabs: &Tabs,
    rect: Rect,
    metrics: &DockMetrics,
    minimum_sizes: &mut HashMap<NodeId, Size>,
    output: &mut LayoutRects,
) -> Result<(), ComputeRectsError> {
    let collapsed = tabs.collapsed;
    let minimum_size = minimum_node_size(layout, id, metrics, minimum_sizes)?;
    // Geometry and resize commands share Layout::minimum_node_size, including
    // the effective tab-strip floor for collapsed groups.
    let minimum_height = minimum_size.height;
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
        Rect::new(rect.origin, Size::new(rect.size.width, minimum_size.height))
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
        let tab_count = tabs.children.len();
        let needs_overflow =
            tab_count as f64 * f64::from(metrics.min_tab_width) > f64::from(group_rect.size.width);

        if !needs_overflow {
            // All tabs remain visible and share the strip width. This naturally
            // compresses headers as more tabs are added, down to min_tab_width.
            let tab_width = group_rect.size.width / tab_count as f32;
            for (index, child_id) in tabs.children.iter().enumerate() {
                if let Some(Node::Panel(panel)) = layout.node(*child_id) {
                    let x = group_rect.origin.x + tab_width * index as f32;
                    let width = if index + 1 == tab_count {
                        (group_rect.origin.x + group_rect.size.width) - x
                    } else {
                        tab_width
                    };
                    let header_rect =
                        Rect::new(Point::new(x, strip_y), Size::new(width, strip_height));
                    output.tab_headers.push(TabHeaderRect::new(
                        id,
                        panel.panel_id,
                        index,
                        header_rect,
                    ));
                }
            }
        } else {
            // This is the project's chosen overflow UX, not a claim about native
            // Photoshop behavior: reserve a trailing button and expose omitted
            // panel ids so the adapter can build a menu/list.
            let requested_button_width =
                metrics.tab_overflow_button_width.min(group_rect.size.width);
            let requested_visible_width = (group_rect.size.width - requested_button_width).max(0.0);
            let visible_count =
                ((requested_visible_width / metrics.min_tab_width).floor() as usize).min(tab_count);
            // When even one minimum-width tab cannot fit beside the button,
            // let the button occupy the whole strip instead of leaving a blank gap.
            let (visible_width, button_width) = if visible_count == 0 {
                (0.0, group_rect.size.width)
            } else {
                (requested_visible_width, requested_button_width)
            };

            let visible_indices: Vec<usize> = if visible_count == 0 {
                Vec::new()
            } else {
                // Keep the active tab visible by selecting a contiguous window
                // that contains it instead of always exposing only the oldest tabs.
                let start = tabs
                    .active_index
                    .saturating_sub(visible_count - 1)
                    .min(tab_count - visible_count);
                (start..start + visible_count).collect()
            };
            if visible_count > 0 {
                let tab_width = visible_width / visible_count as f32;
                for (display_index, child_index) in visible_indices.iter().copied().enumerate() {
                    let child_id = tabs.children[child_index];
                    if let Some(Node::Panel(panel)) = layout.node(child_id) {
                        let x = group_rect.origin.x + tab_width * display_index as f32;
                        let width = if display_index + 1 == visible_count {
                            (group_rect.origin.x + visible_width) - x
                        } else {
                            tab_width
                        };
                        let header_rect =
                            Rect::new(Point::new(x, strip_y), Size::new(width, strip_height));
                        output.tab_headers.push(TabHeaderRect::new(
                            id,
                            panel.panel_id,
                            child_index,
                            header_rect,
                        ));
                    }
                }
            }

            let button_rect = Rect::new(
                Point::new(group_rect.origin.x + visible_width, strip_y),
                Size::new(button_width, strip_height),
            );
            button_rect
                .validate()
                .map_err(ComputeRectsError::Geometry)?;
            let hidden_tabs = tabs
                .children
                .iter()
                .enumerate()
                .filter(|(index, _)| !visible_indices.contains(index))
                .filter_map(|(_, child_id)| match layout.node(*child_id) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if !hidden_tabs.is_empty() {
                output
                    .tab_overflows
                    .push(TabOverflowRect::new(id, button_rect, hidden_tabs));
            }
        }
    }

    if collapsed {
        return Ok(());
    }

    let content_y = group_rect.origin.y + metrics.header_height + metrics.tab_strip_height;
    let content_height =
        (group_rect.size.height - metrics.header_height - metrics.tab_strip_height).max(0.0);
    let content_rect = Rect::new(
        Point::new(group_rect.origin.x, content_y),
        Size::new(group_rect.size.width, content_height),
    );
    if let Some(active_child) = tabs.active_child() {
        layout_node(layout, active_child, content_rect, metrics, minimum_sizes, output)?;
    }
    Ok(())
}
