use dock_core::{DockSide, FloatingGroupId, Point, Rect};

use crate::window_chain::WindowGridMember;

#[derive(Clone, Copy)]
pub(super) struct FloatingColumnWindowSnapshot {
    pub(super) member: WindowGridMember,
    /// Core owner for permanent Column windows; None during a Main-origin drag.
    pub(super) floating_group: Option<FloatingGroupId>,
    /// Present only for temporary ColumnDragWindow instances created from Main.
    pub(super) main_drag_side: Option<DockSide>,
    /// Initial client bounds, using the main-monitor scale until the HWND exists.
    pub(super) bounds: Rect,
    /// Permanent FloatingGroup origin in physical virtual-desktop pixels; zero for temporary Main columns.
    pub(super) floating_origin: Point,
    /// Core layout origin inside the Floating client area remains logical.
    pub(super) local_origin: Point,
    pub(super) contains_transferred_panels: bool,
}

impl FloatingColumnWindowSnapshot {
    /// Returns the Core owner for a permanent Floating Column snapshot.
    /// Temporary Main-drag columns intentionally have no FloatingGroup yet.
    pub(super) fn floating_group(self) -> FloatingGroupId {
        self.floating_group
            .expect("permanent Floating Column snapshot must have a Core owner")
    }
}
