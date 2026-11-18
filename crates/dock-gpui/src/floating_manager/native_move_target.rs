use dock_core::{DropTarget, FloatingGroupId, Point};

use crate::window_chain::{WindowGridDirection, WindowGridMember};

#[derive(Clone, Copy)]
pub(super) enum NativeMoveTarget {
    /// The pointer was released over an unsupported/foreign managed surface.
    /// A Group HWND drag must return to its original slot instead of tearing off.
    RejectedGroupDrop,
    MainDock {
        target: DropTarget,
        point: Point,
    },
    FloatingDock {
        target: DropTarget,
        point: Point,
    },
    FloatingChain {
        anchor: FloatingGroupId,
        anchor_member: WindowGridMember,
        direction: WindowGridDirection,
    },
}
