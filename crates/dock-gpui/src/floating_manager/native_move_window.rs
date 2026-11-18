use dock_core::FloatingGroupId;
use platform::{NativeWindowId, PhysicalPoint};

use crate::window_chain::WindowGridMember;

#[derive(Clone, Copy)]
pub(super) struct NativeMoveWindow {
    pub(super) group: FloatingGroupId,
    pub(super) member: WindowGridMember,
    pub(super) native_window: NativeWindowId,
    /// Actual outer-window origin used as the drag baseline.
    pub(super) drag_origin: PhysicalPoint,
    /// Original origin restored when the gesture is cancelled.
    pub(super) cancel_origin: PhysicalPoint,
}
