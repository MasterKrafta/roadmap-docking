use crate::{DockSide, NodeId, PanelId, Rect};

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

/// Semantic kind of a drop affordance. Cardinal directions are also used by
/// GroupEdge targets; the other variants describe insertion and area targets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropZone {
    /// Place the dragged group before/after a target group in a stack, or split
    /// a generic-profile group horizontally.
    Left,
    Right,
    Top,
    Bottom,
    /// Center of a group: add a dragged panel as a tab or merge a whole group.
    Center,
    /// Position in a tab strip, resolved to an exact insertion/reorder index.
    TabInsertion,
    /// Left/right side of a main column, creating an adjacent column.
    ColumnLeft,
    ColumnRight,
    /// Append a group to the end of an expanded column.
    ColumnEnd,
    /// Outer edge of a dock side.
    DockEdge,
    /// Floating-window center and stack edges.
    FloatingCenter,
    FloatingTop,
    FloatingBottom,
    /// Empty Canvas area, which detaches the dragged node into a floating area.
    FreeArea,
    /// Exact insertion/reorder slot in the flattened order of an icon rail.
    RailInsertion,
}

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

impl DropTarget {
    /// Returns the node directly referenced by this target.
    ///
    /// Area-only targets have no referenced node. Keeping this mapping here
    /// gives validation and drag/drop resolution one source of truth.
    pub(crate) fn referenced_node_id(self) -> Option<NodeId> {
        match self {
            Self::Tabs { group, .. }
            | Self::GroupTabs { group, .. }
            | Self::GroupEdge { group, .. } => Some(group),
            Self::ColumnSlot { column, .. } | Self::RailSlot { column, .. } => Some(column),
            Self::MainColumn { .. } | Self::FreeArea => None,
        }
    }
}

/// Ephemeral overlay geometry paired with its authoritative semantic destination.
///
/// `target` contains stable model IDs and insertion indices; render adapters consume
/// these values directly instead of building a second copy of the layout tree.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DropZoneRect {
    pub target: DropTarget,
    pub zone: DropZone,
    pub rect: Rect,
}

impl DropZoneRect {
    pub const fn new(target: DropTarget, zone: DropZone, rect: Rect) -> Self {
        Self { target, zone, rect }
    }
}
#[cfg(test)]
mod drop_zone_tests {
    use super::DropZone;

    #[test]
    fn semantic_zones_are_distinct() {
        assert_ne!(DropZone::Left, DropZone::Right);
        assert_ne!(DropZone::Top, DropZone::Bottom);
        assert_ne!(DropZone::Center, DropZone::TabInsertion);
        assert_ne!(DropZone::FloatingTop, DropZone::FloatingBottom);
        assert_ne!(DropZone::DockEdge, DropZone::FreeArea);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_zone_kinds() {
        for zone in [
            DropZone::Left,
            DropZone::Right,
            DropZone::Top,
            DropZone::Bottom,
            DropZone::Center,
            DropZone::TabInsertion,
            DropZone::ColumnLeft,
            DropZone::ColumnRight,
            DropZone::ColumnEnd,
            DropZone::DockEdge,
            DropZone::FloatingCenter,
            DropZone::FloatingTop,
            DropZone::FloatingBottom,
            DropZone::FreeArea,
            DropZone::RailInsertion,
        ] {
            let encoded = serde_json::to_string(&zone).unwrap();
            assert_eq!(serde_json::from_str::<DropZone>(&encoded).unwrap(), zone);
        }
    }
}

#[cfg(test)]
mod drop_target_tests {
    use super::DropTarget;
    use super::DropZone;
    use crate::{DockSide, NodeId};
    use slotmap::Key;

    #[test]
    fn referenced_node_id_uses_the_group_or_column_and_omits_area_targets() {
        let mut nodes = slotmap::SlotMap::<NodeId, ()>::with_key();
        let group = nodes.insert(());
        let column = nodes.insert(());

        assert_eq!(
            DropTarget::Tabs { group, index: 2 }.referenced_node_id(),
            Some(group)
        );
        assert_eq!(
            DropTarget::GroupTabs { group, index: 2 }.referenced_node_id(),
            Some(group)
        );
        assert_eq!(
            DropTarget::GroupEdge { group, zone: DropZone::Top }.referenced_node_id(),
            Some(group)
        );
        assert_eq!(
            DropTarget::ColumnSlot { column, index: 2 }.referenced_node_id(),
            Some(column)
        );
        assert_eq!(
            DropTarget::RailSlot { column, index: 2 }.referenced_node_id(),
            Some(column)
        );
        assert_eq!(
            DropTarget::MainColumn { side: DockSide::Right, index: 0 }.referenced_node_id(),
            None
        );
        assert_eq!(DropTarget::FreeArea.referenced_node_id(), None);
    }

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
