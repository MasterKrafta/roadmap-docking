use dock_core::{ChromeMode, FlyoutState, Rect};
use gpui::WindowId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RectCacheKey {
    pub(super) window_id: Option<WindowId>,
    pub(super) layout_version: u64,
    pub(super) viewport: Rect,
    pub(super) metrics_revision: u64,
    pub(super) chrome: ChromeMode,
    pub(super) flyout: Option<FlyoutState>,
    /// ResizeFlyout is transient and does not advance Layout::version().
    /// Its remembered width therefore participates in the geometry cache key.
    pub(super) flyout_width: Option<f32>,
}
