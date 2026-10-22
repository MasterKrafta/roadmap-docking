use crate::{DockSide, DropZone, NodeId};

/// A semantic destination resolved by the geometry/drop-target pipeline.
///
/// Insertion indices are slots: zero is before the first item and the current
/// item count means append. FreeArea is a UI hint; apply it with DockCommand::Float
/// and concrete bounds, not with a docking command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropTarget {
    /// Insert one panel as a tab in an existing group at the exact tab index.
    Tabs { group: NodeId, index: usize },
    /// Merge all tabs from a dragged group into another Tabs group.
    GroupTabs { group: NodeId, index: usize },
    /// Place a new group at an edge of an existing group.
    GroupEdge { group: NodeId, zone: DropZone },
    /// Insert a new main-dock column at an exact position on the given side.
    MainColumn { side: DockSide, index: usize },
    /// Insert a group into an expanded main-dock column at an exact group slot.
    ColumnSlot { column: NodeId, index: usize },
    /// Insert a group into an icon rail at an exact slot in flattened rail order.
    RailSlot { column: NodeId, index: usize },
    /// The central Canvas is empty drop area; adapter should create floating bounds.
    FreeArea,
}

#[cfg(test)]
mod tests {
    use super::DropTarget;
    use crate::{DockSide, DropZone, NodeId};
    use slotmap::Key;

    #[test]
    fn targets_preserve_explicit_positions_and_semantics() {
        assert_ne!(
            DropTarget::MainColumn {
                side: DockSide::Right,
                index: 1
            },
            DropTarget::MainColumn {
                side: DockSide::Right,
                index: 2
            }
        );
        assert_ne!(
            DropTarget::ColumnSlot {
                column: NodeId::null(),
                index: 1
            },
            DropTarget::ColumnSlot {
                column: NodeId::null(),
                index: 2
            }
        );
        assert_ne!(
            DropTarget::RailSlot {
                column: NodeId::null(),
                index: 1
            },
            DropTarget::RailSlot {
                column: NodeId::null(),
                index: 2
            }
        );
        assert_ne!(
            DropTarget::GroupEdge {
                group: NodeId::null(),
                zone: DropZone::Top
            },
            DropTarget::GroupEdge {
                group: NodeId::null(),
                zone: DropZone::Bottom
            }
        );
        assert_ne!(
            DropTarget::Tabs {
                group: NodeId::null(),
                index: 0
            },
            DropTarget::GroupTabs {
                group: NodeId::null(),
                index: 0
            }
        );
        let _free_area = DropTarget::FreeArea;
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_target_and_position() {
        let target = DropTarget::RailSlot {
            column: NodeId::null(),
            index: 3,
        };
        let encoded = serde_json::to_string(&target).unwrap();
        assert_eq!(
            serde_json::from_str::<DropTarget>(&encoded).unwrap(),
            target
        );
    }
}
