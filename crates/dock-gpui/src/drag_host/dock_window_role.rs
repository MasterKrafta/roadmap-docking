use dock_core::FloatingGroupId;

use super::{DockWindowPart, DragHostId};

/// Describes the runtime purpose of a native window used by the dock adapter.
///
/// This is not a Core model type. In particular, `Dragging` identifies a
/// temporary transport/presentation window and does not imply that it owns a
/// dock tree or a `FloatingGroup`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DockWindowRole {
    /// The application's persistent main window.
    Main,
    /// A persistent native window hosting one Core floating composition.
    Floating {
        /// Core identity of the composition hosted by this window.
        group: FloatingGroupId,
    },
    /// A permanent Header or Group window within a Floating composition.
    FloatingPart {
        /// Core identity of the containing composition.
        group: FloatingGroupId,
        /// Identity of the exact Header or Group HWND under the pointer.
        part: DockWindowPart,
    },
    /// A temporary window used only while a dock object is being dragged.
    Dragging {
        /// Runtime identity of the temporary drag host.
        id: DragHostId,
    },
}
