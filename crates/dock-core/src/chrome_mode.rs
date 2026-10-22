/// Persistent visibility mode for application chrome around the canvas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChromeMode {
    /// Dock, toolbar, and Options bar are shown.
    Visible,
    /// Only the dock is hidden; toolbar and Options bar remain visible.
    DockHidden,
    /// Dock, toolbar, and Options bar are hidden.
    #[cfg_attr(feature = "serde", serde(alias = "Hidden"))]
    AllHidden,
}

impl ChromeMode {
    /// Whether main docks and their flyouts should be included in geometry.
    pub const fn dock_visible(self) -> bool {
        matches!(self, Self::Visible)
    }

    /// Whether the application toolbar should be shown by the UI adapter.
    pub const fn toolbar_visible(self) -> bool {
        !matches!(self, Self::AllHidden)
    }

    /// Whether the Options bar should be shown by the UI adapter.
    pub const fn options_bar_visible(self) -> bool {
        !matches!(self, Self::AllHidden)
    }

    /// Returns the target mode for a Tab shortcut.
    ///
    /// Input adapters should apply the returned mode with DockCommand::SetChrome.
    pub const fn after_tab_shortcut(self) -> Self {
        match self {
            Self::AllHidden => Self::Visible,
            Self::Visible | Self::DockHidden => Self::AllHidden,
        }
    }

    /// Returns the target mode for a Shift+Tab shortcut.
    ///
    /// Input adapters should apply the returned mode with DockCommand::SetChrome.
    pub const fn after_shift_tab_shortcut(self) -> Self {
        match self {
            Self::DockHidden => Self::Visible,
            Self::Visible | Self::AllHidden => Self::DockHidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ChromeMode;

    #[test]
    fn visibility_modes_have_distinct_chrome_contracts() {
        assert!(ChromeMode::Visible.dock_visible());
        assert!(ChromeMode::Visible.toolbar_visible());
        assert!(ChromeMode::Visible.options_bar_visible());

        assert!(!ChromeMode::DockHidden.dock_visible());
        assert!(ChromeMode::DockHidden.toolbar_visible());
        assert!(ChromeMode::DockHidden.options_bar_visible());

        assert!(!ChromeMode::AllHidden.dock_visible());
        assert!(!ChromeMode::AllHidden.toolbar_visible());
        assert!(!ChromeMode::AllHidden.options_bar_visible());
    }

    #[test]
    fn tab_shortcut_toggles_all_chrome_and_keeps_other_mode_explicit() {
        assert_eq!(
            ChromeMode::Visible.after_tab_shortcut(),
            ChromeMode::AllHidden
        );
        assert_eq!(
            ChromeMode::AllHidden.after_tab_shortcut(),
            ChromeMode::Visible
        );
        assert_eq!(
            ChromeMode::DockHidden.after_tab_shortcut(),
            ChromeMode::AllHidden
        );
    }

    #[test]
    fn shift_tab_shortcut_toggles_only_dock_visibility() {
        assert_eq!(
            ChromeMode::Visible.after_shift_tab_shortcut(),
            ChromeMode::DockHidden
        );
        assert_eq!(
            ChromeMode::DockHidden.after_shift_tab_shortcut(),
            ChromeMode::Visible
        );
        assert_eq!(
            ChromeMode::AllHidden.after_shift_tab_shortcut(),
            ChromeMode::DockHidden
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn legacy_hidden_name_remains_deserializable_as_all_hidden() {
        assert_eq!(
            serde_json::from_str::<ChromeMode>("\"Hidden\"").unwrap(),
            ChromeMode::AllHidden,
        );
    }
}
