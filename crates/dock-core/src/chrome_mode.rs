/// Persistent visibility mode for application chrome around the canvas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChromeMode {
    /// Application chrome is shown.
    Visible,
    /// Application chrome is hidden to provide more canvas space.
    Hidden,
}

#[cfg(test)]
mod tests {
    use super::ChromeMode;

    #[test]
    fn visibility_modes_are_distinct() {
        assert_ne!(ChromeMode::Visible, ChromeMode::Hidden);
    }
}
