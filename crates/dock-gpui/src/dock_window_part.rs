use dock_core::NodeId;

/// Identifies one permanent part-window when a Floating composition is split into HWNDs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DockWindowPart {
    /// Header window belonging to one Core Column.
    Header { column: NodeId },
    /// Content window belonging to one Core Group.
    Group { group: NodeId },
}
