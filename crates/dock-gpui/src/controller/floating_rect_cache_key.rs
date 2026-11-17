use dock_core::{ChromeMode, FloatingGroupId};
use gpui::WindowId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FloatingRectCacheKey {
    pub(super) window_id: WindowId,
    pub(super) group: FloatingGroupId,
    pub(super) layout_version: u64,
    pub(super) metrics_revision: u64,
    pub(super) chrome: ChromeMode,
}
