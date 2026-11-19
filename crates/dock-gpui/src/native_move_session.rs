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
    /// Exact Floating Column HWND whose Header initiated a whole-WindowGrid move.
    pub(super) source_column_host: Option<NodeId>,
}
