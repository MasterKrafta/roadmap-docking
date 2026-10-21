/// Axis used by split nodes and geometry/layout algorithms.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Axis {
    /// Arrange items from left to right.
    X,
    /// Arrange items from top to bottom.
    Y,
}

#[cfg(test)]
mod tests {
    use super::Axis;

    #[test]
    fn x_and_y_are_distinct() {
        assert_ne!(Axis::X, Axis::Y);
    }
}
