use dock_core::{
    Axis, CommandValue, DockCommand, DockSide, HitResult, Node, NodeId, Point, ResizeEdge,
    Rect, OUTER_COLUMN_EDGE_BOUNDARY_INDEX,
};
use gpui::{App, Entity};

use crate::DockController;

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

/// Applies the latest pointer position. Boundary deltas are measured from the
/// pointer-down origin and compensated by the total effective delta accepted
/// by Core, preventing min/max clamping from accumulating drift.
pub(crate) fn update_resize(
    controller: &Entity<DockController>,
    point: Point,
    app: &mut App,
) {
    let command = controller.update(app, |controller, _| {
        if !controller.ensure_pointer_interaction_current() {
            return None;
        }
        let interaction = controller.resize_interaction()?;
        command_for_pointer(controller, interaction, point)
    });
    let Some(command) = command else {
        return;
    };

    let result = controller.update(app, |controller, cx| {
        let result = if matches!(&command, DockCommand::ResizeFlyout { .. }) {
            controller.dispatch_resize_transient_in(command, cx)
        } else {
            controller.dispatch_resize_in(command, cx)
        };
        if let Ok(outcome) = &result {
            match outcome.actual_value.as_ref() {
                Some(CommandValue::GroupBoundaryDelta(delta))
                | Some(CommandValue::ColumnBoundaryDelta(delta)) => {
                    controller.record_resize_applied_delta(*delta);
                }
                Some(CommandValue::ColumnWidth(width)) => {
                    controller.record_resize_actual_width(*width);
                }
                _ => {}
            }
        }
        result
    });
    if let Err(error) = result {
        tracing::debug!(%error, "splitter resize command was rejected");
    }
}

/// Finishes the gesture. A double click resets the relevant Core ratios;
/// an ordinary pointer-up flushes the last position before committing history.
pub(crate) fn finish_resize(
    controller: &Entity<DockController>,
    point: Point,
    click_count: usize,
    app: &mut App,
) {
    if !controller.update(app, |controller, _| controller.ensure_pointer_interaction_current()) {
        return;
    }
    if click_count == 2 {
        let reset = controller.update(app, |controller, _| {
            let interaction = controller.resize_interaction()?;
            reset_command_for_interaction(controller, interaction)
        });
        if let Some(command) = reset {
            let result = controller.update(app, |controller, cx| {
                controller.dispatch_resize_in(command, cx)
            });
            if let Err(error) = result {
                tracing::debug!(%error, "could not reset splitter sizes");
            }
        }
    } else {
        update_resize(controller, point, app);
    }

    let _ = controller.update(app, |controller, _| controller.end_resize_interaction());
}

fn command_for_pointer(
    controller: &mut DockController,
    interaction: crate::ResizeInteraction,
    point: Point,
) -> Option<DockCommand> {
    let desired_total = match interaction.hit {
        HitResult::Splitter { axis: Axis::X, .. } => point.x - interaction.origin.x,
        HitResult::Splitter { axis: Axis::Y, .. } => point.y - interaction.origin.y,
        HitResult::FloatingResizeEdge { .. } | HitResult::FlyoutResizeEdge { .. } => 0.0,
        _ => return None,
    };
    let delta = incremental_delta(desired_total, interaction.applied_delta);

    match interaction.hit {
        HitResult::FlyoutResizeEdge { panel, edge, .. } => {
            let direction = match edge {
                ResizeEdge::Left => -1.0,
                ResizeEdge::Right => 1.0,
                _ => return None,
            };
            let width = interaction.start_width?
                + direction * (point.x - interaction.origin.x);
            width.is_finite().then_some(DockCommand::ResizeFlyout {
                panel,
                width: width.max(1.0),
            })
        }
        HitResult::FloatingResizeEdge { group, edge } => {
            let dx = point.x - interaction.origin.x;
            let dy = point.y - interaction.origin.y;
            let bounds = resize_floating_bounds(interaction.start_bounds?, edge, dx, dy)?;
            Some(DockCommand::ResizeFloatingBounds { group, bounds })
        }
        HitResult::Splitter {
            owner,
            boundary_index,
            axis,
        } => match axis {
            Axis::Y => Some(DockCommand::ResizeGroups {
                column: owner,
                boundary_index,
                delta,
                available_height: available_group_height(controller, owner)?,
            }),
            Axis::X if boundary_index == OUTER_COLUMN_EDGE_BOUNDARY_INDEX => {
                let side = main_edge_side(controller.layout(), owner)?;
                let direction_delta = signed_outer_width_delta(
                    point.x - interaction.origin.x,
                    side == DockSide::Left,
                );
                let target_width = interaction.start_width? + direction_delta;
                if interaction.actual_width == Some(target_width) {
                    None
                } else {
                    Some(DockCommand::ResizeColumn {
                        column: owner,
                        width: target_width,
                    })
                }
            }
            Axis::X => {
                let side = main_split_side(controller.layout(), owner);
                if let Some(side) = side {
                    Some(DockCommand::ResizeColumnBoundary {
                        side,
                        boundary_index,
                        delta,
                    })
                } else if matches!(
                    controller.layout().node(owner),
                    Some(Node::Split(split)) if split.axis == Axis::X
                ) {
                    Some(DockCommand::ResizeSplitColumnBoundary {
                        split: owner,
                        boundary_index,
                        delta,
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

fn incremental_delta(desired_total: f32, applied_total: f32) -> f32 {
    desired_total - applied_total
}

fn signed_outer_width_delta(pointer_delta: f32, left_dock: bool) -> f32 {
    if left_dock { pointer_delta } else { -pointer_delta }
}

#[cfg(test)]
mod tests {
    use dock_core::{Point, Rect, ResizeEdge, Size};

    use super::{
        incremental_delta, resize_floating_bounds, signed_outer_width_delta,
    };

    #[test]
    fn incremental_delta_tracks_the_effective_core_movement() {
        assert_eq!(incremental_delta(-120.0, -32.0), -88.0);
        assert_eq!(incremental_delta(-32.0, -32.0), 0.0);
        assert_eq!(incremental_delta(-12.0, -32.0), 20.0);
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
