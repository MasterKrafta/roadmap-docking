use dock_core::{DockSide, FloatingGroupId, Point, Rect};

use crate::{components::floating::FloatingPartKind, window_chain::WindowGridMember};

#[derive(Clone, Copy)]
pub(super) struct FloatingPartWindowSnapshot {
    pub(super) member: WindowGridMember,
    /// Core owner for permanent Column windows; None during a Main-origin drag.
    pub(super) floating_group: Option<FloatingGroupId>,
    pub(super) kind: FloatingPartKind,
    /// Present only for temporary Column hosts created from a Main DockArea drag.
    pub(super) main_drag_side: Option<DockSide>,
    /// Initial client bounds, using the main-monitor scale until the HWND exists.
    pub(super) bounds: Rect,
    /// Permanent FloatingGroup origin in physical virtual-desktop pixels; zero for temporary Main parts.
    pub(super) floating_origin: Point,
    /// Core layout origin inside the Floating client area remains logical.
    pub(super) local_origin: Point,
    pub(super) contains_transferred_panels: bool,
}

impl FloatingPartWindowSnapshot {
    /// Returns the Core owner for a permanent Floating Column snapshot.
    /// Temporary Main-drag parts intentionally have no FloatingGroup yet.
    pub(super) fn floating_group(self) -> FloatingGroupId {
        self.floating_group
            .expect("permanent Floating part snapshot must have a Core owner")
    }
}
