use crate::ids::{NodeId, PanelId};

/// Layout item being detached into an in-window floating group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FloatSource {
    Panel(PanelId),
    Group(NodeId),
}
