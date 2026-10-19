use crate::geometry::Rect;
use crate::ids::{FloatingGroupId, NodeId};
use crate::model::DockSide;

use super::{ColumnRef, DropZone};

/// A resolved destination for moving or opening a panel.
///
/// Geometry and hit-testing produce these values; command application validates
/// that the selected target still exists and is compatible with the active profile.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropTarget {
    /// Drop into or beside a regular dock group.
    Group {
        group: NodeId,
        zone: DropZone,
    },
    /// Insert at a particular position in a group's tab strip.
    TabInsertion {
        group: NodeId,
        index: usize,
    },
    /// Drop against a particular column edge.
    Column {
        column: ColumnRef,
        zone: DropZone,
    },
    /// Drop at the outer edge of a dock area.
    DockEdge {
        side: DockSide,
    },
    /// Drop into an existing floating group.
    FloatingGroup {
        floating: FloatingGroupId,
        zone: DropZone,
    },
    /// Drop in free space and create a floating group at these logical bounds.
    FreeArea {
        bounds: Rect,
    },
}
