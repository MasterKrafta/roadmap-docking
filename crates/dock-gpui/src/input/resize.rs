use dock_core::{
    Axis, CommandValue, DockCommand, DockSide, HitResult, Node, NodeId, Point, SplitterRect,
    OUTER_COLUMN_EDGE_BOUNDARY_INDEX,
};
use gpui::{App, Entity};

use crate::DockController;

/// Starts one pointer gesture and its matching single-step history transaction.
pub(crate) fn begin_resize(
    controller: &Entity<DockController>,
    splitter: SplitterRect,
    point: Point,
    app: &mut App,
) {
    let hit = HitResult::Splitter {
        owner: splitter.owner,
        boundary_index: splitter.boundary_index,
        axis: splitter.axis,
    };
    if let Err(error) = controller.update(app, |controller, _| {
        controller.begin_resize_interaction(hit, point)
    }) {
        tracing::debug!(%error, "could not begin splitter resize");
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
        let interaction = controller.resize_interaction()?;
        command_for_pointer(controller, interaction, point)
    });
    let Some(command) = command else {
        return;
    };

    let result = controller.update(app, |controller, cx| {
        let result = controller.dispatch_in(command, cx);
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
    if click_count >= 2 {
        let reset = controller.update(app, |controller, _| {
            let interaction = controller.resize_interaction()?;
            reset_command_for_interaction(controller, interaction)
        });
        if let Some(command) = reset {
            let result = controller.update(app, |controller, cx| {
                controller.dispatch_in(command, cx)
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
    let HitResult::Splitter {
        owner,
        boundary_index,
        axis,
    } = interaction.hit
    else {
        return None;
    };

    let desired_total = match axis {
        Axis::X => point.x - interaction.origin.x,
        Axis::Y => point.y - interaction.origin.y,
    };
    let delta = incremental_delta(desired_total, interaction.applied_delta);

    match axis {
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
            Some(DockCommand::ResizeColumn {
                column: owner,
                width: interaction.start_width? + direction_delta,
            })
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
    }
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
    use super::{incremental_delta, signed_outer_width_delta};

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
}
