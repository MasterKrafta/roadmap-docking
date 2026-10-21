use crate::{FloatingGroupId, NodeId, PanelId};

/// The semantic model object carried by a drag session.
///
/// These identifiers are runtime handles. The command/layout layer must
/// revalidate them when a drop is applied; this transient payload is not a
/// persistent identity or a second copy of the layout tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragPayload {
    /// One panel instance, preserving its runtime identity.
    Panel { panel: PanelId },
    /// A whole tab/group node.
    Group { group: NodeId },
    /// A floating group/window as a single drag source.
    FloatingGroup { group: FloatingGroupId },
}
