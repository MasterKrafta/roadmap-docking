use dock_core::{FloatingGroupId, HitResult, NodeId, Point, Rect};
use gpui::WindowId;

/// The target, pointer origin, and source geometry captured at pointer-down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeInteraction {
    /// The native GPUI window that owns this pointer gesture.
    pub window_id: WindowId,
    /// Present only when the splitter belongs to a permanent Floating Column HWND.
    pub floating_column: Option<(FloatingGroupId, NodeId)>,
    pub hit: HitResult,
    pub origin: Point,
    /// Latest pointer position used only to render a visual preview.
    pub preview_point: Option<Point>,
    pub layout_version: u64,
    pub metrics_revision: u64,
    /// Requested width captured at pointer-down for column/flyout resizing.
    pub start_width: Option<f32>,
    /// Original splitter rectangle, captured before the gesture moves it.
    pub start_splitter_rect: Option<Rect>,
    /// Original flyout rectangle, captured before the gesture moves it.
    pub start_flyout_rect: Option<Rect>,
}
