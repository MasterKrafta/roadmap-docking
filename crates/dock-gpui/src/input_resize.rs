use dock_core::{
    Axis, DockCommand, DockSide, HitResult, LayoutRects, Node, NodeId, OUTER_COLUMN_EDGE_BOUNDARY_INDEX, Point,
    Rect, ResizeEdge, Size,
};
use gpui::{App, Entity, WindowId};

use crate::{DockController, ResizeInteraction};

/// Starts a Core resize transaction from Main geometry.
pub(crate) fn begin_resize_hit(
    controller: &Entity<DockController>,
    window_id: WindowId,
    hit: HitResult,
    point: Point,
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, _| {
        controller.begin_resize_interaction_with_rects(window_id, hit, point, None)
    }) {
        tracing::debug!(%error, "could not begin dock resize");
    }
}

/// Starts a resize from the geometry snapshot belonging to a Floating host.
pub(crate) fn begin_resize_hit_with_rects(
    controller: &Entity<DockController>,
    window_id: WindowId,
    hit: HitResult,
    point: Point,
    rects: &LayoutRects,
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, _| {
        controller.begin_resize_interaction_with_rects(window_id, hit, point, Some(rects))
    }) {
        tracing::debug!(%error, "could not begin floating dock resize");
    }
}

/// Updates the transient preview point only. The Layout is not changed until
/// pointer-up, so a complete gesture produces at most one resize command.
pub(crate) fn update_resize(
    controller: &Entity<DockController>,
    window_id: WindowId,
    point: Point,
    app: &mut App,
) {
    controller.update(app, |controller, cx| {
        if controller.resize_interaction_for_window(window_id).is_none()
            || !controller.ensure_pointer_interaction_current()
        {
            return;
        }
        if controller.set_resize_preview_point(window_id, point) {
            cx.notify();
        }
    });
}

/// Commits exactly one command at pointer-up using Main geometry.
pub(crate) fn finish_resize(
    controller: &Entity<DockController>,
    window_id: WindowId,
    point: Point,
    click_count: usize,
    app: &mut App,
) {
    finish_resize_with_geometry(controller, window_id, point, click_count, None, app);
}

/// Commits a resize using the geometry snapshot belonging to a Floating host.
pub(crate) fn finish_resize_with_rects(
    controller: &Entity<DockController>,
    window_id: WindowId,
    point: Point,
    click_count: usize,
    rects: &LayoutRects,
    app: &mut App,
) {
    finish_resize_with_geometry(controller, window_id, point, click_count, Some(rects), app);
}

fn finish_resize_with_geometry(
    controller: &Entity<DockController>,
    window_id: WindowId,
    point: Point,
    click_count: usize,
    rects: Option<&LayoutRects>,
    app: &mut App,
) {
    let owns_interaction = controller.update(app, |controller, _| {
        controller.resize_interaction_for_window(window_id).is_some()
    });
    if !owns_interaction {
        return;
    }

    let command = controller.update(app, |controller, _| {
        if controller.resize_interaction_for_window(window_id).is_none()
            || !controller.ensure_pointer_interaction_current()
        {
            return None;
        }
        let interaction = controller.resize_interaction_for_window(window_id)?;
        if click_count == 2 {
            reset_command_for_interaction(controller, interaction, rects)
        } else {
            command_for_pointer(controller, interaction, point, rects)
        }
    });

    if let Some(command) = command {
        let result = controller.update(app, |controller, cx| {
            if matches!(&command, DockCommand::ResizeFlyout { .. }) {
                controller.dispatch_resize_transient_in(command, cx)
            } else {
                controller.dispatch_resize_in(command, cx)
            }
        });
        if let Err(error) = result {
            tracing::debug!(%error, "dock resize command was rejected");
        }
    }

    controller.update(app, |controller, cx| {
        controller.end_resize_interaction();
        cx.notify();
    });
}

fn command_for_pointer(
    controller: &mut DockController,
    interaction: ResizeInteraction,
    point: Point,
    rects: Option<&LayoutRects>,
) -> Option<DockCommand> {
    let dx = pointer_delta_from_origin(interaction.origin, point, Axis::X);
    let dy = pointer_delta_from_origin(interaction.origin, point, Axis::Y);

    match interaction.hit {
        HitResult::FlyoutResizeEdge { panel, edge, .. } => {
            let direction = match edge {
                ResizeEdge::Left => -1.0,
                ResizeEdge::Right => 1.0,
                _ => return None,
            };
            let start_width = interaction.start_width?;
            let width = start_width + direction * dx;
            (width.is_finite() && width != start_width).then_some(DockCommand::ResizeFlyout {
                panel,
                width: width.max(1.0),
            })
        }
        HitResult::Splitter {
            owner,
            boundary_index,
            axis,
        } => match axis {
            Axis::Y => {
                if dy == 0.0 {
                    return None;
                }
                Some(DockCommand::ResizeGroups {
                    column: owner,
                    boundary_index,
                    delta: dy,
                    available_height: available_group_height(controller, owner, rects)?,
                })
            }
            Axis::X if boundary_index == OUTER_COLUMN_EDGE_BOUNDARY_INDEX => {
                let side = main_edge_side(controller.layout(), owner)?;
                let start_width = interaction.start_width?;
                let target_width =
                    start_width + signed_outer_width_delta(dx, side == DockSide::Left);
                if !target_width.is_finite() || target_width == start_width {
                    None
                } else {
                    Some(DockCommand::ResizeColumn {
                        column: owner,
                        width: target_width,
                    })
                }
            }
            Axis::X => {
                if dx == 0.0 {
                    return None;
                }
                let side = main_split_side(controller.layout(), owner);
                if let Some(side) = side {
                    Some(DockCommand::ResizeColumnBoundary {
                        side,
                        boundary_index,
                        delta: dx,
                    })
                } else if matches!(
                    controller.layout().node(owner),
                    Some(Node::Split(split)) if split.axis == Axis::X
                ) {
                    Some(DockCommand::ResizeSplitColumnBoundary {
                        split: owner,
                        boundary_index,
                        delta: dx,
                    })
                } else {
                    None
                }
            }
        },
        _ => None,
    }
}

fn reset_command_for_interaction(
    controller: &mut DockController,
    interaction: crate::ResizeInteraction,
    rects: Option<&LayoutRects>,
) -> Option<DockCommand> {
    let HitResult::Splitter {
        owner,
        axis,
        boundary_index,
    } = interaction.hit
    else {
        return None;
    };

    match axis {
        Axis::Y => Some(DockCommand::ResetColumnRatios {
            column: owner,
            available_height: available_group_height(controller, owner, rects)?,
        }),
        Axis::X => {
            let side = if boundary_index == OUTER_COLUMN_EDGE_BOUNDARY_INDEX {
                main_edge_side(controller.layout(), owner)
            } else {
                main_split_side(controller.layout(), owner)
            }?;
            Some(DockCommand::ResetMainColumnRatios { side })
        }
    }
}

fn main_split_side(layout: &dock_core::Layout, owner: NodeId) -> Option<DockSide> {
    [DockSide::Left, DockSide::Right].into_iter().find(|side| {
        layout
            .main_area(*side)
            .is_some_and(|area| area.root == owner)
    })
}

fn main_edge_side(layout: &dock_core::Layout, owner: NodeId) -> Option<DockSide> {
    [DockSide::Left, DockSide::Right]
        .into_iter()
        .find(|side| outer_column(layout, *side) == Some(owner))
}

fn outer_column(layout: &dock_core::Layout, side: DockSide) -> Option<NodeId> {
    let root = layout.main_area(side)?.root;
    match layout.node(root)? {
        Node::Column(_) => Some(root),
        Node::Split(split) if split.axis == Axis::X => {
            let child = match side {
                DockSide::Left => split.children.last(),
                DockSide::Right => split.children.first(),
            }?;
            matches!(layout.node(*child), Some(Node::Column(_))).then_some(*child)
        }
        _ => None,
    }
}

fn available_group_height(
    controller: &mut DockController,
    column: NodeId,
    host_rects: Option<&LayoutRects>,
) -> Option<f32> {
    let metrics = controller.metrics();
    let total_height = {
        let rects = match host_rects {
            Some(rects) => rects,
            None => controller.geometry().ok()?,
        };
        rects
            .vertical_overflow
            .iter()
            .find(|overflow| overflow.owner == column)
            .map(|overflow| overflow.content_extent.size.height)
            .or_else(|| {
                rects
                    .columns
                    .iter()
                    .find(|rect| rect.column == column)
                    .map(|rect| rect.rect.size.height)
            })?
    };
    let group_count = match controller.layout().node(column)? {
        Node::Column(value) => value.groups.len(),
        _ => return None,
    };
    if group_count < 2 {
        return None;
    }
    let gaps = metrics.splitter_thickness * group_count.saturating_sub(1) as f32;
    let available = total_height - gaps;
    (available.is_finite() && available > 0.0).then_some(available)
}

fn pointer_delta_from_origin(origin: Point, current: Point, axis: Axis) -> f32 {
    match axis {
        Axis::X => current.x - origin.x,
        Axis::Y => current.y - origin.y,
    }
}

fn signed_outer_width_delta(pointer_delta: f32, left_dock: bool) -> f32 {
    if left_dock {
        pointer_delta
    } else {
        -pointer_delta
    }
}

/// Visual-only geometry for the active resize. A splitter preview is a thin
/// guide line; flyout resizing uses an outline of the future bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResizePreviewKind {
    SplitterLine,
    BoundsOutline,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResizePreview {
    pub rect: Rect,
    pub kind: ResizePreviewKind,
    /// A column scroll owner whose content coordinates must be translated.
    pub scroll_owner: Option<NodeId>,
    /// Identifies the original splitter whose static line is hidden during drag.
    pub splitter: Option<(NodeId, usize, Axis)>,
}

pub(crate) fn resize_preview_for_interaction(
    interaction: ResizeInteraction,
    window_id: WindowId,
) -> Option<ResizePreview> {
    if interaction.window_id != window_id {
        return None;
    }
    let point = interaction.preview_point?;
    let dx = pointer_delta_from_origin(interaction.origin, point, Axis::X);
    let dy = pointer_delta_from_origin(interaction.origin, point, Axis::Y);

    match interaction.hit {
        HitResult::Splitter {
            owner,
            boundary_index,
            axis,
        } => {
            let start = interaction.start_splitter_rect?;
            let rect = splitter_preview_rect(start, axis, dx, dy)?;
            Some(ResizePreview {
                rect,
                kind: ResizePreviewKind::SplitterLine,
                scroll_owner: (axis == Axis::Y).then_some(owner),
                splitter: Some((owner, boundary_index, axis)),
            })
        }
        HitResult::FlyoutResizeEdge { edge, .. } => {
            let start = interaction.start_flyout_rect?;
            let direction = match edge {
                ResizeEdge::Left => -1.0,
                ResizeEdge::Right => 1.0,
                _ => return None,
            };
            let width = interaction.start_width? + direction * dx;
            if !width.is_finite() || width <= 0.0 {
                return None;
            }
            let right = start.origin.x + start.size.width;
            let left = if edge == ResizeEdge::Left {
                right - width.max(1.0)
            } else {
                start.origin.x
            };
            let rect = Rect::new(
                Point::new(left, start.origin.y),
                Size::new(width.max(1.0), start.size.height),
            );
            rect.validate_non_empty().ok()?;
            Some(ResizePreview {
                rect,
                kind: ResizePreviewKind::BoundsOutline,
                scroll_owner: None,
                splitter: None,
            })
        }
        // Only splitter and flyout-edge hits can start a resize.
        // Keep the preview API exhaustive as HitResult grows with new hit kinds.
        _ => None,
    }
}

fn splitter_preview_rect(start: Rect, axis: Axis, dx: f32, dy: f32) -> Option<Rect> {
    let preview_thickness = 3.0;
    let rect = match axis {
        Axis::X => Rect::new(
            Point::new(
                start.origin.x + (start.size.width - preview_thickness) * 0.5 + dx,
                start.origin.y,
            ),
            Size::new(preview_thickness, start.size.height),
        ),
        Axis::Y => Rect::new(
            Point::new(
                start.origin.x,
                start.origin.y + (start.size.height - preview_thickness) * 0.5 + dy,
            ),
            Size::new(start.size.width, preview_thickness),
        ),
    };
    rect.validate_non_empty().ok()?;
    Some(rect)
}

#[cfg(test)]
mod tests {
    use dock_core::{Axis, Point, Rect, Size};

    use super::{
        pointer_delta_from_origin, signed_outer_width_delta,
        splitter_preview_rect,
    };

    #[test]
    fn pointer_delta_is_always_measured_from_the_original_press_point() {
        let origin = Point::new(100.0, 80.0);

        assert_eq!(
            pointer_delta_from_origin(origin, Point::new(140.0, 80.0), Axis::X),
            40.0
        );
        assert_eq!(
            pointer_delta_from_origin(origin, Point::new(112.0, 80.0), Axis::X),
            12.0
        );
        assert_eq!(
            pointer_delta_from_origin(origin, Point::new(100.0, 50.0), Axis::Y),
            -30.0
        );
    }

    #[test]
    fn splitter_preview_is_centered_in_the_hit_zone_and_uses_the_full_delta() {
        let original = Rect::new(Point::new(200.0, 40.0), Size::new(8.0, 300.0));

        assert_eq!(
            splitter_preview_rect(original, Axis::X, 24.0, 0.0),
            Some(Rect::new(Point::new(226.5, 40.0), Size::new(3.0, 300.0),))
        );

        let horizontal = Rect::new(Point::new(40.0, 200.0), Size::new(300.0, 8.0));
        assert_eq!(
            splitter_preview_rect(horizontal, Axis::Y, 0.0, 24.0),
            Some(Rect::new(Point::new(40.0, 226.5), Size::new(300.0, 3.0),))
        );
    }

    #[test]
    fn left_and_right_dock_edges_use_opposite_width_directions() {
        assert_eq!(signed_outer_width_delta(24.0, true), 24.0);
        assert_eq!(signed_outer_width_delta(24.0, false), -24.0);
    }

}
