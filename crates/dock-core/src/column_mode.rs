/// Presentation mode of a Photoshop-style dock column.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ColumnMode {
    /// Full-width column with expanded group headers and contents.
    Expanded,
    /// Narrow icon rail; groups remain in the tree and can be opened as flyouts.
    IconsOnly,
}

#[cfg(test)]
mod tests {
    use super::ColumnMode;

    #[test]
    fn expanded_and_rail_modes_are_distinct() {
        assert_ne!(ColumnMode::Expanded, ColumnMode::IconsOnly);
    }
}
