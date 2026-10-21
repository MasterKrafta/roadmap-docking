use crate::{DockSide, DropZone, NodeId};

/// A fully specified destination for a docking operation.
///
/// Indexes are insertion slots: zero is before the first item, and the current
/// item count means append. Rail drops use a dedicated target so inserting
/// between icons never depends on pointer proximity or inferred ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropTarget {
    /// Insert a panel as a tab in this group at the exact tab index.
    Tabs { group: NodeId, index: usize },
    /// Place a new group at an edge of an existing group.
    GroupEdge { group: NodeId, zone: DropZone },
    /// Insert a main-dock column at an exact position on the given side.
    MainColumn { side: DockSide, index: usize },
    /// Insert a new group into an icon rail at the exact group slot.
    ///
    /// The target column must be in IconsOnly mode. This is intentionally
    /// separate from GroupEdge, which targets the edge of a group body.
    RailSlot { column: NodeId, index: usize },
}

#[cfg(test)]
mod tests {
    use super::DropTarget;
    use crate::{DockSide, DropZone, NodeId};

    #[test]
    fn targets_preserve_explicit_positions() {
        assert_eq!(
            DropTarget::MainColumn { side: DockSide::Right, index: 2 },
            DropTarget::MainColumn { side: DockSide::Right, index: 2 }
        );
        assert_ne!(
            DropTarget::RailSlot { column: NodeId::null(), index: 1 },
            DropTarget::RailSlot { column: NodeId::null(), index: 2 }
        );
        assert_ne!(
            DropTarget::GroupEdge { group: NodeId::null(), zone: DropZone::Top },
            DropTarget::GroupEdge { group: NodeId::null(), zone: DropZone::Bottom }
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_target_and_position() {
        let target = DropTarget::RailSlot { column: NodeId::null(), index: 3 };
        let encoded = serde_json::to_string(&target).unwrap();
        assert_eq!(serde_json::from_str::<DropTarget>(&encoded).unwrap(), target);
    }
}
