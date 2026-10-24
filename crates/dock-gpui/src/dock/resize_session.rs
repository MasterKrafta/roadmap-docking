use dock_core::{Axis, DockMetrics, Layout, LayoutRects, Node, Point, Rect};
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
    InvalidHandle,
    MissingColumnGeometry,
    MissingFloatingGeometry,
    MissingFloatingEdge,
    SessionAlreadyActive,
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

    pub(super) fn pointer_start(&self) -> Point {
        self.pointer_start
    }

    pub(super) fn start_geometry(&self) -> StartGeometry {
        self.start_geometry
    }
}

fn capture_start_geometry(
    layout: &Layout,
    rects: &LayoutRects,
    metrics: &DockMetrics,
    handle: ResizeHandle,
) -> Result<StartGeometry, ResizeStartError> {
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
            if !available_height.is_finite() {
                return Err(ResizeStartError::MissingColumnGeometry);
            }

            Ok(StartGeometry::VerticalGroups {
                available_height: available_height.max(0.0),
            })
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
        ResizeHandle::OuterColumn { column, .. } => {
            let Some(Node::Column(value)) = layout.node(column) else {
                return Err(ResizeStartError::InvalidHandle);
            };
            if value.mode.is_rail() {
                return Err(ResizeStartError::InvalidHandle);
            }

            let Some(column_rect) = rects.columns.iter().find(|entry| entry.column == column) else {
                return Err(ResizeStartError::MissingColumnGeometry);
            };
            let start_width = column_rect.rect.size.width;
            if !start_width.is_finite() || start_width <= 0.0 {
                return Err(ResizeStartError::MissingColumnGeometry);
            }
            Ok(StartGeometry::OuterColumn { start_width })
        }
        ResizeHandle::FloatingEdge { group, edge } => {
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
            if !rects
                .floating_resize_edges
                .iter()
                .any(|entry| entry.group == group && entry.edge == edge)
            {
                return Err(ResizeStartError::MissingFloatingEdge);
            }
            Ok(StartGeometry::FloatingEdge {
                bounds: floating.rect,
            })
        }
    }
}
