use dock_core::{FloatingGroupId, NodeId, Point};
use gpui::WindowId;

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
    /// The pointer in one permanent Floating Column HWND.
    FloatingColumn {
        group: FloatingGroupId,
        window_id: WindowId,
        column: NodeId,
        point: Point,
    },
}

impl DockingWindowAtPoint {
    pub(crate) const fn role(self) -> Option<crate::DockWindowRole> {
        match self {
            Self::Main { .. } => Some(crate::DockWindowRole::Main),
            Self::FloatingColumn { group, column, .. } => {
                Some(crate::DockWindowRole::FloatingColumn { group, column })
            },
            Self::ForeignWindow => None,
        }
    }
}
