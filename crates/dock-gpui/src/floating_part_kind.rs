use dock_core::NodeId;

/// Identity of a permanent Floating window that renders exactly one Core member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FloatingPartKind {
    /// One native window containing the Header and every Group in a Column.
    Column { column: NodeId },
    /// Temporary Main DockArea Header window used while the legacy part drag is active.
    Header { column: NodeId },
    /// Temporary Group content window used by the legacy Main DockArea drag path.
    Group { column: NodeId, group: NodeId },
}
