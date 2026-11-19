use dock_core::{FloatingGroupId, NodeId};

use super::DragHostId;

/// Describes the runtime purpose of a native window used by the dock adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DockWindowRole {
    /// The application's persistent main window.
    Main,
    /// A legacy native window hosting one Core Floating composition.
    Floating {
        /// Core identity of the composition hosted by this window.
        group: FloatingGroupId,
    },
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
