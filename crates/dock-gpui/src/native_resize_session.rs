use dock_core::{FloatingGroupId, Rect};
use gpui::WindowId;

#[derive(Clone, Copy)]
pub(super) struct NativeResizeSession {
    pub(super) group: FloatingGroupId,
    pub(super) window_id: WindowId,
    pub(super) start_bounds: Rect,
    pub(super) transaction_started: bool,
}
