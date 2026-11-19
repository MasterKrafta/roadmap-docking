use dock_core::{FloatingGroupId, NodeId};

use super::DragHostId;

/// Describes the runtime purpose of a native window used by the dock adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DockWindowRole {
    /// The application's persistent main window.
    Main,
    /// A permanent native window containing one Floating Column.
    FloatingColumn {
        /// Core identity of the containing composition.
        group: FloatingGroupId,
        /// Core identity of the Column hosted by this window.
        column: NodeId,
    },
    /// A temporary window used only while a dock object is being dragged.
    Dragging {
        /// Runtime identity of the temporary drag host.
        id: DragHostId,
    },
}
