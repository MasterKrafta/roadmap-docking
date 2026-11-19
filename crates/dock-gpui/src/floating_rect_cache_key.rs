use dock_core::{ChromeMode, FloatingGroupId, NodeId};
use gpui::WindowId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FloatingRectCacheKey {
    pub(super) window_id: WindowId,
    pub(super) group: FloatingGroupId,
    pub(super) column: Option<NodeId>,
    pub(super) column_width: Option<f32>,
    pub(super) column_height: Option<f32>,
    pub(super) column_origin_y_offset: Option<f32>,
    pub(super) layout_version: u64,
    pub(super) metrics_revision: u64,
    pub(super) chrome: ChromeMode,
}
