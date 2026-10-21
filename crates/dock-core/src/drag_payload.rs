use crate::{FloatingGroupId, NodeId, PanelId};

/// The semantic model object carried by a drag session.
///
/// These identifiers are runtime handles. The command/layout layer must
/// revalidate them when a drop is applied; this transient payload is not a
/// persistent identity or a second copy of the layout tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragPayload {
    /// One panel tab. The request carries only this panel's identity.
    Panel { panel: PanelId },
    /// A whole Tabs group being dragged from its group grip.
    ///
    /// Keep this variant distinct from Panel: every child tab moves together,
    /// and the existing Tabs node retains active_index and collapsed state when
    /// it is moved, split, or floated as a group.
    Group { group: NodeId },
    /// A floating window moved by its window/title drag surface.
    ///
    /// This is distinct from dragging the Tabs group inside that window.
    FloatingGroup { group: FloatingGroupId },
}
