use dock_core::{FloatingGroupId, Rect};
use gpui::WindowId;
use platform::{NativeWindowId, PhysicalSize};

use super::{PanelWindowHandle, PanelWindowVisibility};

pub(super) struct PanelWindowEntry {
    pub(super) window: PanelWindowHandle,
    pub(super) window_id: WindowId,
    pub(super) native_window: Option<NativeWindowId>,
    pub(super) bounds: Rect,
    pub(super) native_min_size: Option<PhysicalSize>,
    pub(super) native_horizontal_resize_min_widths: Option<(i32, i32)>,
    pub(super) native_left_resize_enabled: Option<bool>,
    pub(super) native_top_resize_enabled: Option<bool>,
    pub(super) native_scale_factor: Option<f32>,
    /// The HWND may differ from Core bounds after the OS-owned resize loop.
    /// A final reconciliation must write Core's canonical bounds back.
    pub(super) native_bounds_dirty: bool,
    pub(super) resize_enabled: bool,
    pub(super) host_group: Option<FloatingGroupId>,
    pub(super) visibility: PanelWindowVisibility,
}
