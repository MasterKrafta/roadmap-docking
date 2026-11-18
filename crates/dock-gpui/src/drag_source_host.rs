use dock_core::FloatingGroupId;

use super::DockWindowPart;

/// Identifies the persistent host where dragged dock content originated.
///
/// Main is unique within one DockHost. Floating uses its Core group identity,
/// which remains stable even if the panel view later moves to another host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragSourceHost {
    /// Content originated in the persistent main dock window.
    Main,
    /// Content originated in a persistent floating composition.
    Floating {
        /// Core identity of the source floating composition.
        group: FloatingGroupId,
    },
    /// Content originated in one permanent Header/Group part window.
    FloatingPart {
        /// Core identity of the containing floating composition.
        group: FloatingGroupId,
        /// Exact persistent window that initiated the content gesture.
        part: DockWindowPart,
    },
}
