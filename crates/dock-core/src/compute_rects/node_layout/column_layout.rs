use super::*;

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
