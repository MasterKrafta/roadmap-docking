use dock_core::{
    Axis, DockCommand, DockMetrics, DockSide, Layout, LayoutRects, Node, Point, Rect, ResizeEdge,
};
use crate::requests::ResizeHandle;

/// Geometry measured once at pointer-down and reused for the entire resize.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum StartGeometry {
    VerticalGroups { available_height: f32 },
    HorizontalSplit,
    OuterColumn { start_width: f32 },
    FloatingEdge { bounds: Rect },
}

/// Failure to begin a resize from an invalid handle or incomplete frame snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResizeStartError {
    InvalidPointer,
    StaleFrameGeometry,
    InvalidHandle,
    MissingColumnGeometry,
    MissingFloatingGeometry,
    MissingFloatingEdge,
    SessionAlreadyActive,
}

/// Failure while translating a resize pointer position into a preview command.
#[derive(Clone, Debug)]
pub(crate) enum ResizeUpdateError {
    NoActiveSession,
    InvalidPointer,
    InvalidDelta,
    InvalidStartGeometry,
    InvalidBounds,
    Command(dock_core::CommandError),
}

/// Immutable starting point for a pointer-driven resize gesture.
///
/// The baseline Layout is cloned once when the gesture begins. Subsequent
/// previews can always derive their candidate from that same baseline rather
/// than accumulating rounding errors from one pointer move to the next.
#[derive(Debug)]
pub(crate) struct ResizeSession {
    handle: ResizeHandle,
    baseline: Layout,
    pointer_start: Point,
    start_geometry: StartGeometry,
}

impl ResizeSession {
    pub(super) fn begin(
        layout: &Layout,
        rects: &LayoutRects,
        metrics: &DockMetrics,
        handle: ResizeHandle,
        pointer_start: Point,
    ) -> Result<Self, ResizeStartError> {
        if !pointer_start.is_finite() {
            return Err(ResizeStartError::InvalidPointer);
        }

        let start_geometry = capture_start_geometry(layout, rects, metrics, handle)?;

        Ok(Self {
            handle,
            baseline: layout.clone(),
            pointer_start,
            start_geometry,
        })
    }

    pub(super) fn handle(&self) -> ResizeHandle {
        self.handle
    }

    pub(super) fn baseline(&self) -> &Layout {
        &self.baseline
    }

    /// Returns ownership of the pointer-down snapshot when the session ends.
    pub(super) fn into_baseline(self) -> Layout {
        self.baseline
    }

    pub(super) fn pointer_start(&self) -> Point {
        self.pointer_start
    }

    pub(super) fn start_geometry(&self) -> StartGeometry {
        self.start_geometry
    }

    /// Builds a command from total displacement since pointer-down.
    ///
    /// The baseline and captured geometry never change, so pointer updates are
    /// independent of the order or number of preceding move events.
    pub(super) fn command_for_pointer(
        &self,
        pointer: Point,
    ) -> Result<DockCommand, ResizeUpdateError> {
        if !pointer.is_finite() {
            return Err(ResizeUpdateError::InvalidPointer);
        }

        let delta_x = pointer.x - self.pointer_start.x;
        let delta_y = pointer.y - self.pointer_start.y;

        match (self.handle, self.start_geometry) {
            (
                ResizeHandle::Splitter {
                    owner,
                    boundary_index,
                    axis: Axis::Y,
                },
                StartGeometry::VerticalGroups { available_height },
            ) if available_height.is_finite() && available_height > 0.0 => {
                if !delta_y.is_finite() {
                    return Err(ResizeUpdateError::InvalidDelta);
                }
                Ok(DockCommand::ResizeGroups {
                    column: owner,
                    boundary_index,
                    delta: delta_y,
                    available_height,
                })
            }
            (
                ResizeHandle::Splitter {
                    owner,
                    boundary_index,
                    axis: Axis::X,
                },
                StartGeometry::HorizontalSplit,
            ) => {
                if !delta_x.is_finite() {
                    return Err(ResizeUpdateError::InvalidDelta);
                }
                if self
                    .baseline
                    .main_area(DockSide::Left)
                    .is_some_and(|area| area.root == owner)
                {
                    Ok(DockCommand::ResizeColumnBoundary {
                        side: DockSide::Left,
                        boundary_index,
                        delta: delta_x,
                    })
                } else if self
                    .baseline
                    .main_area(DockSide::Right)
                    .is_some_and(|area| area.root == owner)
                {
                    Ok(DockCommand::ResizeColumnBoundary {
                        side: DockSide::Right,
                        boundary_index,
                        delta: delta_x,
                    })
                } else {
                    Ok(DockCommand::ResizeSplitColumnBoundary {
                        split: owner,
                        boundary_index,
                        delta: delta_x,
                    })
                }
            }
            (
                ResizeHandle::OuterColumn { column, side },
                StartGeometry::OuterColumn { start_width },
            ) => {
                if !delta_x.is_finite() {
                    return Err(ResizeUpdateError::InvalidDelta);
                }
                let width = match side {
                    DockSide::Left => start_width + delta_x,
                    DockSide::Right => start_width - delta_x,
                };
                if !width.is_finite() || width <= 0.0 {
                    return Err(ResizeUpdateError::InvalidBounds);
                }
                Ok(DockCommand::ResizeColumn { column, width })
            }
            (
                ResizeHandle::FloatingEdge { group, edge },
                StartGeometry::FloatingEdge { bounds },
            ) => {
                let uses_x = matches!(
                    edge,
                    ResizeEdge::Left
                        | ResizeEdge::TopLeft
                        | ResizeEdge::BottomLeft
                        | ResizeEdge::Right
                        | ResizeEdge::TopRight
                        | ResizeEdge::BottomRight
                );
                let uses_y = matches!(
                    edge,
                    ResizeEdge::Top
                        | ResizeEdge::TopLeft
                        | ResizeEdge::TopRight
                        | ResizeEdge::Bottom
                        | ResizeEdge::BottomLeft
                        | ResizeEdge::BottomRight
                );
                if (uses_x && !delta_x.is_finite()) || (uses_y && !delta_y.is_finite()) {
                    return Err(ResizeUpdateError::InvalidDelta);
                }
                let bounds = resized_floating_bounds(bounds, edge, delta_x, delta_y);
                if bounds.validate_non_empty().is_err() {
                    return Err(ResizeUpdateError::InvalidBounds);
                }
                Ok(DockCommand::ResizeFloatingBounds { group, bounds })
            }
            _ => Err(ResizeUpdateError::InvalidStartGeometry),
        }
    }
}

fn resized_floating_bounds(
    mut bounds: Rect,
    edge: ResizeEdge,
    delta_x: f32,
    delta_y: f32,
) -> Rect {
    if matches!(edge, ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft) {
        bounds.origin.x += delta_x;
        bounds.size.width -= delta_x;
    }
    if matches!(edge, ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight) {
        bounds.size.width += delta_x;
    }
    if matches!(edge, ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight) {
        bounds.origin.y += delta_y;
        bounds.size.height -= delta_y;
    }
    if matches!(edge, ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight) {
        bounds.size.height += delta_y;
    }
    bounds
}

fn capture_start_geometry(
    layout: &Layout,
    rects: &LayoutRects,
    metrics: &DockMetrics,
    handle: ResizeHandle,
) -> Result<StartGeometry, ResizeStartError> {
    if let ResizeHandle::Splitter {
        owner,
        boundary_index,
        axis,
    } = handle
    {
        let has_splitter_geometry = rects.splitters.iter().any(|entry| {
            entry.owner == owner
                && entry.boundary_index == boundary_index
                && entry.axis == axis
                && entry.rect.validate_non_empty().is_ok()
        });
        if !has_splitter_geometry {
            return Err(ResizeStartError::InvalidHandle);
        }
    }

    match handle {
        ResizeHandle::Splitter {
            owner,
            boundary_index,
            axis: Axis::Y,
        } => {
            let Some(Node::Column(column)) = layout.node(owner) else {
                return Err(ResizeStartError::InvalidHandle);
            };
            let group_count = column.groups.len();
            if boundary_index >= group_count.saturating_sub(1) {
                return Err(ResizeStartError::InvalidHandle);
            }

            let Some(column_rect) = rects.columns.iter().find(|entry| entry.column == owner) else {
                return Err(ResizeStartError::MissingColumnGeometry);
            };
            let available_height = column_rect.rect.size.height
                - metrics.splitter_thickness * group_count.saturating_sub(1) as f32;
            if !available_height.is_finite() || available_height <= 0.0 {
                return Err(ResizeStartError::MissingColumnGeometry);
            }

            Ok(StartGeometry::VerticalGroups { available_height })
        }
        ResizeHandle::Splitter {
            owner,
            boundary_index,
            axis: Axis::X,
        } => {
            let Some(Node::Split(split)) = layout.node(owner) else {
                return Err(ResizeStartError::InvalidHandle);
            };
            if split.axis != Axis::X
                || boundary_index >= split.children.len().saturating_sub(1)
            {
                return Err(ResizeStartError::InvalidHandle);
            }
            Ok(StartGeometry::HorizontalSplit)
        }
        ResizeHandle::OuterColumn { column, side } => {
            let columns = crate::tree_query::main_column_ids(layout, side);
            let expected_outer_column = match side {
                DockSide::Left => columns.last().copied(),
                DockSide::Right => columns.first().copied(),
            };
            if expected_outer_column != Some(column) {
                return Err(ResizeStartError::InvalidHandle);
            }

            let Some(Node::Column(value)) = layout.node(column) else {
                return Err(ResizeStartError::InvalidHandle);
            };
            if value.mode.is_rail() {
                return Err(ResizeStartError::InvalidHandle);
            }

            let Some(column_rect) = rects.columns.iter().find(|entry| entry.column == column) else {
                return Err(ResizeStartError::MissingColumnGeometry);
            };
            if column_rect.rect.validate_non_empty().is_err() {
                return Err(ResizeStartError::MissingColumnGeometry);
            }
            let start_width = column_rect.rect.size.width;
            Ok(StartGeometry::OuterColumn { start_width })
        }
        ResizeHandle::FloatingEdge { group, edge } => {
            if layout.floating_group(group).is_none() {
                return Err(ResizeStartError::MissingFloatingGeometry);
            }

            let Some(floating) = rects
                .floating_groups
                .iter()
                .find(|entry| entry.group == group)
            else {
                return Err(ResizeStartError::MissingFloatingGeometry);
            };
            if floating.rect.validate_non_empty().is_err() {
                return Err(ResizeStartError::MissingFloatingGeometry);
            }
            let Some(edge_geometry) = rects
                .floating_resize_edges
                .iter()
                .find(|entry| entry.group == group && entry.edge == edge)
            else {
                return Err(ResizeStartError::MissingFloatingEdge);
            };
            if edge_geometry.rect.validate_non_empty().is_err() {
                return Err(ResizeStartError::MissingFloatingEdge);
            }
            Ok(StartGeometry::FloatingEdge {
                bounds: floating.rect,
            })
        }
    }
}


#[cfg(test)]
mod tests {
    use super::{capture_start_geometry, ResizeSession, ResizeStartError, ResizeUpdateError, StartGeometry};
    use crate::dock::DockController;
    use crate::requests::ResizeHandle;
    use dock_core::{Axis, DockCommand, DockSide, Node, Point, Rect, ResizeEdge, Size};
    use slotmap::Key;
    use ui::Theme;

    fn session(
        layout: &dock_core::Layout,
        handle: ResizeHandle,
        start_geometry: StartGeometry,
    ) -> ResizeSession {
        ResizeSession {
            handle,
            baseline: layout.clone(),
            pointer_start: Point::new(100.0, 100.0),
            start_geometry,
        }
    }

    #[test]
    fn vertical_splitter_uses_absolute_y_delta_and_keeps_zero_delta() {
        let layout = DockController::new_initial(&Theme::default()).unwrap().layout().clone();
        let owner = layout
            .nodes()
            .find_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
            .unwrap();
        let handle = ResizeHandle::Splitter {
            owner,
            boundary_index: 0,
            axis: Axis::Y,
        };
        let session = session(
            &layout,
            handle,
            StartGeometry::VerticalGroups {
                available_height: 640.0,
            },
        );

        assert_eq!(
            session.command_for_pointer(Point::new(123.0, 117.5)).unwrap(),
            DockCommand::ResizeGroups {
                column: owner,
                boundary_index: 0,
                delta: 17.5,
                available_height: 640.0,
            }
        );
        assert_eq!(
            session.command_for_pointer(Point::new(100.0, 100.0)).unwrap(),
            DockCommand::ResizeGroups {
                column: owner,
                boundary_index: 0,
                delta: 0.0,
                available_height: 640.0,
            }
        );
    }

    #[test]
    fn horizontal_splitter_maps_main_root_to_its_actual_side() {
        let layout = DockController::new_initial(&Theme::default()).unwrap().layout().clone();
        let (side, owner) = [DockSide::Left, DockSide::Right]
            .into_iter()
            .find_map(|side| {
                let owner = layout.main_area(side)?.root;
                matches!(layout.node(owner), Some(Node::Split(split)) if split.axis == Axis::X)
                    .then_some((side, owner))
            })
            .expect("initial layout should have a horizontal main-area root");
        let session = session(
            &layout,
            ResizeHandle::Splitter {
                owner,
                boundary_index: 0,
                axis: Axis::X,
            },
            StartGeometry::HorizontalSplit,
        );

        assert_eq!(
            session.command_for_pointer(Point::new(112.0, 130.0)).unwrap(),
            DockCommand::ResizeColumnBoundary {
                side,
                boundary_index: 0,
                delta: 12.0,
            }
        );
    }

    #[test]
    fn outer_column_resize_requires_matching_outer_side() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let layout = controller.layout().clone();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();
        let mut checked_any = false;

        for side in [DockSide::Left, DockSide::Right] {
            let columns = crate::tree_query::main_column_ids(&layout, side);
            let column = match side {
                DockSide::Left => columns.last().copied(),
                DockSide::Right => columns.first().copied(),
            };
            let Some(column) = column else {
                continue;
            };
            if !matches!(layout.node(column), Some(Node::Column(value)) if !value.mode.is_rail())
                || !frame.rects().columns.iter().any(|entry| entry.column == column)
            {
                continue;
            }

            assert!(matches!(
                capture_start_geometry(
                    &layout,
                    frame.rects(),
                    controller.metrics(),
                    ResizeHandle::OuterColumn { column, side },
                ),
                Ok(StartGeometry::OuterColumn { .. })
            ));

            let wrong_side = match side {
                DockSide::Left => DockSide::Right,
                DockSide::Right => DockSide::Left,
            };
            assert_eq!(
                capture_start_geometry(
                    &layout,
                    frame.rects(),
                    controller.metrics(),
                    ResizeHandle::OuterColumn {
                        column,
                        side: wrong_side,
                    },
                ),
                Err(ResizeStartError::InvalidHandle)
            );
            checked_any = true;
        }

        assert!(checked_any, "initial layout should expose a resizable outer column");
    }

    #[test]
    fn outer_column_resize_rejects_invalid_column_rect() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let layout = controller.layout().clone();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();
        let (column, side) = [DockSide::Left, DockSide::Right]
            .into_iter()
            .find_map(|side| {
                let columns = crate::tree_query::main_column_ids(&layout, side);
                let column = match side {
                    DockSide::Left => columns.last().copied(),
                    DockSide::Right => columns.first().copied(),
                }?;
                (matches!(layout.node(column), Some(Node::Column(value)) if !value.mode.is_rail())
                    && frame.rects().columns.iter().any(|entry| entry.column == column))
                .then_some((column, side))
            })
            .expect("initial layout should expose a resizable outer column");
        let mut rects = frame.rects().clone();
        let column_rect = rects
            .columns
            .iter_mut()
            .find(|entry| entry.column == column)
            .expect("outer column geometry should exist");
        column_rect.rect.size.height = 0.0;

        assert_eq!(
            capture_start_geometry(
                &layout,
                &rects,
                controller.metrics(),
                ResizeHandle::OuterColumn { column, side },
            ),
            Err(ResizeStartError::MissingColumnGeometry)
        );
    }

    #[test]
    fn outer_columns_use_opposite_width_delta_and_floating_edges_keep_the_opposite_corner() {
        let layout = DockController::new_initial(&Theme::default()).unwrap().layout().clone();
        let column = layout
            .nodes()
            .find_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
            .unwrap();

        for (side, width) in [(DockSide::Left, 300.0), (DockSide::Right, 260.0)] {
            let session = session(
                &layout,
                ResizeHandle::OuterColumn { column, side },
                StartGeometry::OuterColumn { start_width: 280.0 },
            );
            assert_eq!(
                session.command_for_pointer(Point::new(120.0, 100.0)).unwrap(),
                DockCommand::ResizeColumn { column, width }
            );
        }

        let floating = session(
            &layout,
            ResizeHandle::FloatingEdge {
                group: dock_core::FloatingGroupId::null(),
                edge: ResizeEdge::TopLeft,
            },
            StartGeometry::FloatingEdge {
                bounds: Rect::new(Point::new(10.0, 20.0), Size::new(300.0, 200.0)),
            },
        );
        assert_eq!(
            floating.command_for_pointer(Point::new(120.0, 130.0)).unwrap(),
            DockCommand::ResizeFloatingBounds {
                group: dock_core::FloatingGroupId::null(),
                bounds: Rect::new(Point::new(30.0, 50.0), Size::new(280.0, 170.0)),
            }
        );
        assert!(matches!(
            floating.command_for_pointer(Point::new(500.0, 500.0)),
            Err(ResizeUpdateError::InvalidBounds)
        ));
    }

    #[test]
    fn command_for_pointer_rejects_non_finite_coordinates() {
        let layout = DockController::new_initial(&Theme::default()).unwrap().layout().clone();
        let owner = layout
            .nodes()
            .find_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
            .unwrap();
        let session = session(
            &layout,
            ResizeHandle::Splitter {
                owner,
                boundary_index: 0,
                axis: Axis::Y,
            },
            StartGeometry::VerticalGroups {
                available_height: 640.0,
            },
        );

        assert!(matches!(
            session.command_for_pointer(Point::new(f32::NAN, 120.0)),
            Err(ResizeUpdateError::InvalidPointer)
        ));
        assert!(matches!(
            session.command_for_pointer(Point::new(120.0, f32::INFINITY)),
            Err(ResizeUpdateError::InvalidPointer)
        ));
    }


    #[test]
    fn command_for_pointer_rejects_overflowing_used_delta() {
        let layout = DockController::new_initial(&Theme::default()).unwrap().layout().clone();
        let owner = layout
            .nodes()
            .find_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
            .unwrap();
        let mut session = session(
            &layout,
            ResizeHandle::Splitter {
                owner,
                boundary_index: 0,
                axis: Axis::Y,
            },
            StartGeometry::VerticalGroups {
                available_height: 640.0,
            },
        );
        session.pointer_start = Point::new(100.0, f32::MAX);

        assert!(matches!(
            session.command_for_pointer(Point::new(100.0, -f32::MAX)),
            Err(ResizeUpdateError::InvalidDelta)
        ));
    }

    #[test]
    fn command_for_pointer_ignores_overflowing_unused_axis_delta() {
        let layout = DockController::new_initial(&Theme::default()).unwrap().layout().clone();
        let owner = layout
            .nodes()
            .find_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
            .unwrap();
        let mut session = session(
            &layout,
            ResizeHandle::Splitter {
                owner,
                boundary_index: 0,
                axis: Axis::Y,
            },
            StartGeometry::VerticalGroups {
                available_height: 640.0,
            },
        );
        session.pointer_start = Point::new(f32::MAX, 100.0);

        assert_eq!(
            session
                .command_for_pointer(Point::new(-f32::MAX, 110.0))
                .unwrap(),
            DockCommand::ResizeGroups {
                column: owner,
                boundary_index: 0,
                delta: 10.0,
                available_height: 640.0,
            }
        );
    }

    #[test]
    fn splitter_resize_requires_matching_frame_geometry() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let layout = controller.layout().clone();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();

        for axis in [Axis::X, Axis::Y] {
            let splitter = *frame
                .rects()
                .splitters
                .iter()
                .find(|splitter| splitter.axis == axis)
                .expect("initial layout should contain a splitter on each axis");
            let handle = ResizeHandle::Splitter {
                owner: splitter.owner,
                boundary_index: splitter.boundary_index,
                axis,
            };
            let mut rects = frame.rects().clone();
            rects.splitters.retain(|entry| {
                entry.owner != splitter.owner
                    || entry.boundary_index != splitter.boundary_index
                    || entry.axis != axis
            });

            assert_eq!(
                capture_start_geometry(&layout, &rects, controller.metrics(), handle),
                Err(ResizeStartError::InvalidHandle)
            );
        }
    }

    #[test]
    fn splitter_resize_rejects_empty_frame_geometry() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let layout = controller.layout().clone();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();

        for axis in [Axis::X, Axis::Y] {
            let splitter = *frame
                .rects()
                .splitters
                .iter()
                .find(|splitter| splitter.axis == axis)
                .expect("initial layout should contain a splitter on each axis");
            let handle = ResizeHandle::Splitter {
                owner: splitter.owner,
                boundary_index: splitter.boundary_index,
                axis,
            };
            let mut rects = frame.rects().clone();
            let geometry = rects
                .splitters
                .iter_mut()
                .find(|entry| {
                    entry.owner == splitter.owner
                        && entry.boundary_index == splitter.boundary_index
                        && entry.axis == axis
                })
                .expect("matching splitter geometry should exist");
            geometry.rect.size.height = 0.0;

            assert_eq!(
                capture_start_geometry(&layout, &rects, controller.metrics(), handle),
                Err(ResizeStartError::InvalidHandle)
            );
        }
    }

    #[test]
    fn floating_resize_requires_group_in_current_layout() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let mut stale_layout = controller.layout().clone();
        let group = stale_layout
            .nodes()
            .find_map(|(id, node)| match node {
                Node::Column(column) if column.groups.len() > 1 => {
                    column.groups.first().copied()
                }
                _ => None,
            })
            .expect("initial layout should contain a column with multiple groups");
        stale_layout
            .apply(
                DockCommand::Float {
                    node: group,
                    bounds: Rect::new(Point::new(80.0, 60.0), Size::new(320.0, 220.0)),
                },
                controller.metrics(),
            )
            .unwrap();
        let (floating_id, _) = stale_layout
            .floating_groups()
            .next()
            .expect("floating command should create a floating group");
        let frame = crate::FrameGeometry::compute(
            &stale_layout,
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            controller.metrics(),
        )
        .unwrap();
        let edge = frame
            .rects()
            .floating_resize_edges
            .iter()
            .find(|entry| entry.group == floating_id)
            .expect("floating frame should contain resize-edge geometry");

        assert!(controller.layout().floating_group(floating_id).is_none());
        assert_eq!(
            capture_start_geometry(
                controller.layout(),
                frame.rects(),
                controller.metrics(),
                ResizeHandle::FloatingEdge {
                    group: floating_id,
                    edge: edge.edge,
                },
            ),
            Err(ResizeStartError::MissingFloatingGeometry)
        );
    }

    #[test]
    fn floating_resize_rejects_empty_edge_geometry() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let mut layout = controller.layout().clone();
        let group = layout
            .nodes()
            .find_map(|(id, node)| match node {
                Node::Column(column) if column.groups.len() > 1 => {
                    column.groups.first().copied()
                }
                _ => None,
            })
            .expect("initial layout should contain a column with multiple groups");
        layout
            .apply(
                DockCommand::Float {
                    node: group,
                    bounds: Rect::new(Point::new(80.0, 60.0), Size::new(320.0, 220.0)),
                },
                controller.metrics(),
            )
            .unwrap();
        let (floating_id, _) = layout
            .floating_groups()
            .next()
            .expect("floating command should create a floating group");
        let frame = crate::FrameGeometry::compute(
            &layout,
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            controller.metrics(),
        )
        .unwrap();
        let edge = *frame
            .rects()
            .floating_resize_edges
            .iter()
            .find(|entry| entry.group == floating_id)
            .expect("floating frame should contain resize-edge geometry");
        let mut rects = frame.rects().clone();
        let edge_geometry = rects
            .floating_resize_edges
            .iter_mut()
            .find(|entry| entry.group == floating_id && entry.edge == edge.edge)
            .expect("matching floating edge geometry should exist");
        edge_geometry.rect.size.width = 0.0;

        assert_eq!(
            capture_start_geometry(
                &layout,
                &rects,
                controller.metrics(),
                ResizeHandle::FloatingEdge {
                    group: floating_id,
                    edge: edge.edge,
                },
            ),
            Err(ResizeStartError::MissingFloatingEdge)
        );
    }

    #[test]
    fn vertical_resize_rejects_zero_available_height_at_start() {
        let controller = DockController::new_initial(&Theme::default()).unwrap();
        let layout = controller.layout().clone();
        let frame = controller
            .frame(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)))
            .unwrap();
        let splitter = frame
            .rects()
            .splitters
            .iter()
            .find(|splitter| splitter.axis == Axis::Y)
            .expect("initial layout should contain a vertical group splitter");
        let group_count = match layout.node(splitter.owner) {
            Some(Node::Column(column)) => column.groups.len(),
            other => panic!("vertical splitter should belong to a column, got {other:?}"),
        };
        let mut rects = frame.rects().clone();
        let column_rect = rects
            .columns
            .iter_mut()
            .find(|column| column.column == splitter.owner)
            .expect("splitter column should have geometry");
        column_rect.rect.size.height =
            controller.metrics().splitter_thickness * (group_count - 1) as f32;

        assert_eq!(
            capture_start_geometry(
                &layout,
                &rects,
                controller.metrics(),
                crate::requests::ResizeHandle::Splitter {
                    owner: splitter.owner,
                    boundary_index: splitter.boundary_index,
                    axis: Axis::Y,
                },
            ),
            Err(ResizeStartError::MissingColumnGeometry)
        );
    }

}
