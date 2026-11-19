use dock_core::{FloatingGroupId, Point};
use gpui::WindowId;

use dock_core::NodeId;

/// A registered persistent DockHost under the physical cursor, together with
/// the pointer converted to that host's Core-local logical coordinate system.
/// The point is always target-host-local; it is not the original drag source point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DockingWindowAtPoint {
    ForeignWindow,
    /// The pointer in MainHost-local logical coordinates.
    Main {
        point: Point,
    },
    /// The pointer in this FloatingHost's local logical coordinates.
    Floating {
        group: FloatingGroupId,
        window_id: WindowId,
        point: Point,
    },
    /// The pointer in one permanent Floating Column HWND.
    FloatingColumn {
        group: FloatingGroupId,
        window_id: WindowId,
        column: NodeId,
        point: Point,
    }
}

impl DockingWindowAtPoint {
    pub(crate) const fn role(self) -> Option<crate::DockWindowRole> {
        match self {
            Self::Main { .. } => Some(crate::DockWindowRole::Main),
            Self::Floating { group, .. } => Some(crate::DockWindowRole::Floating { group }),
            Self::FloatingColumn { group, column, .. } => {
                Some(crate::DockWindowRole::FloatingColumn { group, column })
            },
            Self::ForeignWindow => None,
        }
    }
}
