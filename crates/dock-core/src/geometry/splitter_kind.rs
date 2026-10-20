/// The axis/operation represented by a splitter hit region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitterKind {
    /// A boundary between vertically stacked groups in one column.
    Group,
    /// A boundary between horizontally adjacent columns.
    Column,
}

#[cfg(test)]
mod tests {
    use super::SplitterKind;

    #[test]
    fn splitter_kinds_are_distinct_and_copyable() {
        let group = SplitterKind::Group;
        let column = SplitterKind::Column;
        assert_ne!(group, column);
        assert_eq!(group, SplitterKind::Group);
        assert_eq!(column, SplitterKind::Column);
    }
}
