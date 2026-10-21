/// The semantic item currently being dragged, used to compute legal drop targets.
///
/// A whole Tabs group keeps its identity during reorder/split. Individual panels
/// carry their PanelId so the core can apply the panel's area restrictions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropSource {
    Panel { panel: crate::PanelId },
    Group { group: crate::NodeId },
}
