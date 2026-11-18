use dock_core::{DockSide, FloatingGroupId, Point, Rect};

use crate::{components::floating::FloatingPartKind, window_chain::WindowGridMember};

#[derive(Clone, Copy)]
pub(super) struct FloatingPartWindowSnapshot {
    pub(super) member: WindowGridMember,
    pub(super) floating_group: FloatingGroupId,
    pub(super) kind: FloatingPartKind,
    /// Present only for temporary Header/Group hosts created from a Main DockArea drag.
    pub(super) main_drag_side: Option<DockSide>,
    /// Initial client bounds, using the main-monitor scale until the HWND exists.
    pub(super) bounds: Rect,
    /// Core FloatingGroup origin is already in physical virtual-desktop pixels.
    pub(super) floating_origin: Point,
    /// Core layout origin inside the Floating client area remains logical.
    pub(super) local_origin: Point,
    pub(super) contains_transferred_panels: bool,
}
