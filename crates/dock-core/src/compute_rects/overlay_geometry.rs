use super::*;

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
