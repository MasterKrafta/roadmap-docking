use crate::ids::{FloatingGroupId, NodeId, PanelId};

/// Item being dragged when resolving a docking destination.
///
/// A panel may be detached from a dock tab group or moved into another group.
/// A dock group may be floated as a unit, while a floating group may be docked
/// into a group, tab insertion position, column, or dock edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropPayload {
    Panel(PanelId),
    Group(NodeId),
    FloatingGroup(FloatingGroupId),
}

#[cfg(test)]
mod tests {
    use super::DropPayload;
    use crate::{FloatingGroupId, NodeId, PanelId};
    use slotmap::Key;

    #[test]
    fn payload_distinguishes_dragged_item_kinds() {
        assert_ne!(
            DropPayload::Panel(PanelId::null()),
            DropPayload::Group(NodeId::null())
        );
        assert_ne!(
            DropPayload::Group(NodeId::null()),
            DropPayload::FloatingGroup(FloatingGroupId::null())
        );
    }
}
