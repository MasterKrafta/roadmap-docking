use dock_core::{FloatingGroupId, NodeId};

/// Identifies the persistent host where dragged dock content originated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragSourceHost {
    /// Content originated in the persistent main dock window.
    Main,
    /// Content originated in a legacy composition host.
    Floating {
        /// Core identity of the source floating composition.
        group: FloatingGroupId,
    },
    /// Content originated in one permanent Floating Column HWND.
    FloatingColumn {
        /// Core identity of the containing floating composition.
        group: FloatingGroupId,
        /// Exact Core Column represented by the source HWND.
        column: NodeId,
    },
}
