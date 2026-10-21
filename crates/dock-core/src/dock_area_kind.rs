/// Distinguishes an in-window dock area from a detached floating area.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DockAreaKind {
    /// A dock area attached to the main application window.
    Main,
    /// A dock area hosted by a separate floating window.
    Floating,
}

#[cfg(test)]
mod tests {
    use super::DockAreaKind;

    #[test]
    fn main_and_floating_areas_are_distinct() {
        assert_ne!(DockAreaKind::Main, DockAreaKind::Floating);
    }
}
