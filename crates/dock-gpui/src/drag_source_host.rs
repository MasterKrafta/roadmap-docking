use dock_core::{FloatingGroupId, NodeId};

/// Identifies the persistent host where dragged dock content originated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragSourceHost {
    /// Content originated in the persistent main dock window.
    Main,
    /// Content originated in a legacy composition host.
    FloatingColumn {
        /// Core identity of the containing floating composition.
        group: FloatingGroupId,
        /// Exact Core Column represented by the source HWND.
        column: NodeId,
    },
}
