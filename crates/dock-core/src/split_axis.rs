/// The coordinate axis along which a split arranges its children.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitAxis {
    /// Arrange children along the X axis (left to right).
    X,
    /// Arrange children along the Y axis (top to bottom).
    Y,
}

#[cfg(test)]
mod tests {
    use super::SplitAxis;

    #[test]
    fn split_axes_are_distinct() {
        assert_ne!(SplitAxis::X, SplitAxis::Y);
    }
}
