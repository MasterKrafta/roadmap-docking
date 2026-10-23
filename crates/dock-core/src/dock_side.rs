/// Side of the main window occupied by a dock area.
///
/// The central canvas is not represented as a dock side or as a dock-tree node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DockSide {
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use super::DockSide;

    #[test]
    fn left_and_right_are_distinct() {
        assert_ne!(DockSide::Left, DockSide::Right);
    }
}
