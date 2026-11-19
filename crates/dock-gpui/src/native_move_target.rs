use dock_core::{DropTarget, FloatingGroupId, Point};

use crate::window_chain::{WindowGridDirection, WindowGridMember};

#[derive(Clone, Copy)]
pub(super) enum NativeMoveTarget {
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
