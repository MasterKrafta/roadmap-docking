use dock_core::{FloatingGroupId, Rect};
use gpui::WindowId;

use crate::window_chain::WindowGridMember;

#[derive(Clone, Copy)]
pub(super) struct NativeResizeSession {
    pub(super) group: FloatingGroupId,
    pub(super) member: WindowGridMember,
    pub(super) window_id: WindowId,
    pub(super) start_bounds: Rect,
    pub(super) transaction_started: bool,
}
