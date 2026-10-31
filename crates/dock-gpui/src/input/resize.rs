use dock_core::{
    Axis, DockCommand, DockSide, HitResult, Node, NodeId, Point, ResizeEdge, Rect, Size,
    OUTER_COLUMN_EDGE_BOUNDARY_INDEX,
};
use gpui::{App, Entity};

use crate::{DockController, ResizeInteraction};

/// Starts the same Core resize transaction from either a splitter or a
/// floating-window edge/corner supplied by the geometry hit-test.
pub(crate) fn begin_resize_hit(
    controller: &Entity<DockController>,
    hit: HitResult,
    point: Point,
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, _| {
        controller.begin_resize_interaction(hit, point)
    }) {
        tracing::debug!(%error, "could not begin dock resize");
    }
}

/// Updates the transient preview point only. The Layout is not changed until
/// pointer-up, so a complete gesture produces at most one resize command.
pub(crate) fn update_resize(
    controller: &Entity<DockController>,
    point: Point,
    app: &mut App,
) {
    controller.update(app, |controller, cx| {
        if !controller.ensure_pointer_interaction_current() {
            return;
        }
        if controller.set_resize_preview_point(point) {
            cx.notify();
        }
    });
}

/// Commits exactly one command at pointer-up. A double click keeps the
/// existing reset behavior; all other gestures use the full delta from origin.
pub(crate) fn finish_resize(
    controller: &Entity<DockController>,
    point: Point,
    click_count: usize,
    app: &mut App,
) {
    let command = controller.update(app, |controller, _| {
        if !controller.ensure_pointer_interaction_current() {
            return None;
        }
        let interaction = controller.resize_interaction()?;
        if click_count == 2 {
            reset_command_for_interaction(controller, interaction)
        } else {
            command_for_pointer(controller, interaction, point)
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
            (width.is_finite() && width != start_width).then_some(
                DockCommand::ResizeFlyout {
                    panel,
                    width: width.max(1.0),
                },
            )
        }
        HitResult::FloatingResizeEdge { group, edge } => {
            let start_bounds = interaction.start_bounds?;
            let bounds = resize_floating_bounds(start_bounds, edge, dx, dy)?;
            (bounds != start_bounds)
                .then_some(DockCommand::ResizeFloatingBounds { group, bounds })
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
                    available_height: available_group_height(controller, owner)?,
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

/// Applies an absolute pointer delta to the original floating bounds. The
/// opposite edge remains fixed for left/top resizing; Core performs the final
/// minimum-size and viewport clamping in its next geometry snapshot.
fn resize_floating_bounds(
    start: Rect,
    edge: ResizeEdge,
    dx: f32,
    dy: f32,
) -> Option<Rect> {
    if !dx.is_finite() || !dy.is_finite() {
        return None;
    }

    let original_left = start.origin.x;
    let original_top = start.origin.y;
    let original_right = original_left + start.size.width;
    let original_bottom = original_top + start.size.height;

    let mut left = original_left;
    let mut top = original_top;
    let mut right = original_right;
    let mut bottom = original_bottom;

    match edge {
        ResizeEdge::Left => left = (original_left + dx).min(original_right - 1.0),
        ResizeEdge::Right => right = (original_right + dx).max(original_left + 1.0),
        ResizeEdge::Top => top = (original_top + dy).min(original_bottom - 1.0),
        ResizeEdge::Bottom => bottom = (original_bottom + dy).max(original_top + 1.0),
        ResizeEdge::TopLeft => {
            left = (original_left + dx).min(original_right - 1.0);
            top = (original_top + dy).min(original_bottom - 1.0);
        }
        ResizeEdge::TopRight => {
            right = (original_right + dx).max(original_left + 1.0);
            top = (original_top + dy).min(original_bottom - 1.0);
        }
        ResizeEdge::BottomLeft => {
            left = (original_left + dx).min(original_right - 1.0);
            bottom = (original_bottom + dy).max(original_top + 1.0);
        }
        ResizeEdge::BottomRight => {
            right = (original_right + dx).max(original_left + 1.0);
            bottom = (original_bottom + dy).max(original_top + 1.0);
        }
    }

    let resized = Rect::new(
        Point::new(left, top),
        dock_core::Size::new(right - left, bottom - top),
    );
    resized.validate().ok().map(|_| resized)
}

fn reset_command_for_interaction(
    controller: &mut DockController,
    interaction: crate::ResizeInteraction,
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
            available_height: available_group_height(controller, owner)?,
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
    [DockSide::Left, DockSide::Right]
        .into_iter()
        .find(|side| layout.main_area(*side).is_some_and(|area| area.root == owner))
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

fn available_group_height(controller: &mut DockController, column: NodeId) -> Option<f32> {
    let metrics = controller.metrics();
    let total_height = {
        let rects = controller.geometry().ok()?;
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
    if left_dock { pointer_delta } else { -pointer_delta }
}

/// Visual-only geometry for the active resize. A splitter preview is a thin
/// guide line; floating and flyout resizes use an outline of the future bounds.
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
}

pub(crate) fn resize_preview_for_interaction(
    interaction: ResizeInteraction,
) -> Option<ResizePreview> {
    let point = interaction.preview_point?;
    let dx = pointer_delta_from_origin(interaction.origin, point, Axis::X);
    let dy = pointer_delta_from_origin(interaction.origin, point, Axis::Y);

    match interaction.hit {
        HitResult::Splitter { owner, axis, .. } => {
            let start = interaction.start_splitter_rect?;
            let rect = splitter_preview_rect(start, axis, dx, dy)?;
            Some(ResizePreview {
                rect,
                kind: ResizePreviewKind::SplitterLine,
                scroll_owner: (axis == Axis::Y).then_some(owner),
            })
        }
        HitResult::FloatingResizeEdge { edge, .. } => {
            let rect = resize_floating_bounds(interaction.start_bounds?, edge, dx, dy)?;
            Some(ResizePreview {
                rect,
                kind: ResizePreviewKind::BoundsOutline,
                scroll_owner: None,
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
            })
        }
        // Only splitter, floating-edge, and flyout-edge hits can start a resize.
        // Keep the preview API exhaustive as HitResult grows with new hit kinds.
        _ => None,
    }
}

fn splitter_preview_rect(start: Rect, axis: Axis, dx: f32, dy: f32) -> Option<Rect> {
    let rect = match axis {
        Axis::X => Rect::new(
            Point::new(start.origin.x + dx - 1.5, start.origin.y),
            Size::new(3.0, start.size.height),
        ),
        Axis::Y => Rect::new(
            Point::new(start.origin.x, start.origin.y + dy - 1.5),
            Size::new(start.size.width, 3.0),
        ),
    };
    rect.validate_non_empty().ok()?;
    Some(rect)
}

#[cfg(test)]
mod tests {
    use dock_core::{Axis, Point, Rect, ResizeEdge, Size};

    use super::{
        pointer_delta_from_origin, resize_floating_bounds, signed_outer_width_delta,
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
    fn splitter_preview_uses_the_full_delta_from_its_original_rect() {
        let original = Rect::new(Point::new(200.0, 40.0), Size::new(4.0, 300.0));

        assert_eq!(
            splitter_preview_rect(original, Axis::X, 24.0, 0.0),
            Some(Rect::new(
                Point::new(222.5, 40.0),
                Size::new(3.0, 300.0),
            ))
        );
    }

    #[test]
    fn left_and_right_dock_edges_use_opposite_width_directions() {
        assert_eq!(signed_outer_width_delta(24.0, true), 24.0);
        assert_eq!(signed_outer_width_delta(24.0, false), -24.0);
    }

    #[test]
    fn floating_top_left_resize_keeps_the_opposite_edges_fixed() {
        let original = Rect::new(Point::new(100.0, 50.0), Size::new(300.0, 200.0));

        assert_eq!(
            resize_floating_bounds(original, ResizeEdge::TopLeft, 20.0, 30.0),
            Some(Rect::new(
                Point::new(120.0, 80.0),
                Size::new(280.0, 170.0),
            ))
        );
    }

    #[test]
    fn floating_bottom_right_resize_keeps_the_origin_fixed() {
        let original = Rect::new(Point::new(100.0, 50.0), Size::new(300.0, 200.0));

        assert_eq!(
            resize_floating_bounds(original, ResizeEdge::BottomRight, 40.0, 50.0),
            Some(Rect::new(
                Point::new(100.0, 50.0),
                Size::new(340.0, 250.0),
            ))
        );
    }

    #[test]
    fn floating_resize_rejects_non_finite_pointer_deltas() {
        let original = Rect::new(Point::new(100.0, 50.0), Size::new(300.0, 200.0));
        assert_eq!(
            resize_floating_bounds(original, ResizeEdge::Right, f32::NAN, 0.0),
            None
        );
    }
}
