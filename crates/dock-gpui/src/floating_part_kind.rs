use dock_core::NodeId;

/// Identity of a permanent Floating window that renders exactly one Core member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FloatingPartKind {
    Header { column: NodeId },
    Group { column: NodeId, group: NodeId },
}
