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

#[cfg(test)]
mod tests {
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
