use dock_core::{FloatingGroupId, Rect};
use gpui::WindowId;

use crate::window_chain::WindowGridMember;

#[derive(Clone, Copy)]
pub(super) struct NativeResizeSession {
    pub(super) group: FloatingGroupId,
    pub(super) member: WindowGridMember,
    pub(super) window_id: WindowId,
    pub(super) transaction_started: bool,
    /// Core geometry before any cursor sample in this native resize session.
    pub(super) initial_group_bounds: Rect,
    pub(super) initial_column_bounds: Rect,
    pub(super) scale_factor: f32,
}
