use dock_core::{DockSide, DropTarget, PanelId, Point, Rect};
use gpui::{Subscription, WindowHandle, WindowId};
use platform::{NativeWindowFrameState, NativeWindowId};

use crate::{DockWindowRole, DragHostId};

use super::{DragHostHandoff, DragHostSurface};

pub(super) struct ActiveDragHost {
    pub(super) id: DragHostId,
    pub(super) window: WindowHandle<DragHostSurface>,
    pub(super) window_id: WindowId,
    pub(super) native_window: NativeWindowId,
    pub(super) panels: Vec<PanelId>,
    pub(super) dock_area: Option<DockSide>,
    pub(super) grab_offset: Point,
    pub(super) frame_state: Option<NativeWindowFrameState>,
    /// Remembers whether DragHost began with its native frame removed. A promoted
    /// Group HWND must keep the same borderless style after bounds measurement.
    pub(super) frame_was_removed: bool,
    pub(super) visible: bool,
    pub(super) drop_target: Option<DropTarget>,
    pub(super) target_role: Option<DockWindowRole>,
    pub(super) free_area_bounds: Option<Rect>,
    /// Defers layout publication until permanent Floating part windows are ready.
    pub(super) defer_layout_events_until_parts_ready: bool,
    /// Cursor position captured when the button is released before source detachment.
    pub(super) pending_release_cursor: Option<platform::PhysicalPoint>,
    pub(super) source_detach_subscription: Option<Subscription>,
    pub(super) handoff: Option<DragHostHandoff>,
}
