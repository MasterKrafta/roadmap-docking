use dock_core::{FloatingGroupId, NodeId, Point, Rect};
use platform::{NativeWindowId, PhysicalPoint};

use super::NativeMoveTarget;

#[derive(Clone, Copy)]
pub(super) struct NativeMoveSession {
    pub(super) group: FloatingGroupId,
    pub(super) native_window: NativeWindowId,
    pub(super) hide_header: bool,
    pub(super) initial_cursor: PhysicalPoint,
    pub(super) start_bounds: Rect,
    pub(super) start_pointer: Point,
    pub(super) drag_threshold: f32,
    pub(super) threshold_crossed: bool,
    pub(super) high_resolution_timer: bool,
    pub(super) drop_target: Option<NativeMoveTarget>,
    /// Header HWND that initiated a whole-WindowGrid move.
    pub(super) source_header: Option<NodeId>,
    /// Group HWND that initiated a single-Group move.
    pub(super) source_group: Option<NodeId>,
    /// Original Column for a GroupGrip move.
    pub(super) source_column: Option<NodeId>,
    /// Whether the Group was the only Group in its source Column.
    pub(super) source_column_is_single_group: bool,
}
