/// An edge of a target group used for an unambiguous group split/drop.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropZone {
    /// Place the new group to the left of the target.
    Left,
    /// Place the new group to the right of the target.
    Right,
    /// Place the new group above the target.
    Top,
    /// Place the new group below the target.
    Bottom,
}

#[cfg(test)]
mod tests {
    use super::DropZone;

    #[test]
    fn drop_zones_are_distinct() {
        assert_ne!(DropZone::Left, DropZone::Right);
        assert_ne!(DropZone::Top, DropZone::Bottom);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_drop_zone() {
        for zone in [DropZone::Left, DropZone::Right, DropZone::Top, DropZone::Bottom] {
            let encoded = serde_json::to_string(&zone).unwrap();
            assert_eq!(serde_json::from_str::<DropZone>(&encoded).unwrap(), zone);
        }
    }
}
