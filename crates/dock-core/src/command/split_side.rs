/// Direction in which a panel is split away from an existing group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitSide {
    Top,
    Bottom,
    Left,
    Right,
}
