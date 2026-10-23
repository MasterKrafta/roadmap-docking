/// Opaque 24-bit RGB value consumed by the GPUI adapter.
pub type Rgb = u32;

/// Central color palette. Render components must read colors from this object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    pub window_background: Rgb,
    pub canvas_background: Rgb,
    pub dock_background: Rgb,
    pub group_background: Rgb,
    pub border: Rgb,
    pub splitter: Rgb,
    pub primary_text: Rgb,
    pub muted_text: Rgb,
    pub active_tab: Rgb,
    pub hover: Rgb,
    pub focus: Rgb,
    /// Accent used for the active dock drop preview.
    pub drop_target: Rgb,
    pub toolbar_background: Rgb,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            window_background: 0x1E1E1E,
            canvas_background: 0x121212,
            dock_background: 0x2C2C2C,
            group_background: 0x252525,
            border: 0x383838,
            splitter: 0x444444,
            primary_text: 0xFFFFFF,
            muted_text: 0xB3B3B3,
            active_tab: 0x383838,
            hover: 0x383838,
            focus: 0x0C8CE9,
            drop_target: 0x0C8CE9,
            toolbar_background: 0x2C2C2C,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeColors;

    #[test]
    fn default_palette_matches_figma_dark_colors() {
        let colors = ThemeColors::default();
        assert_eq!(colors.window_background, 0x1E1E1E);
        assert_eq!(colors.canvas_background, 0x121212);
        assert_eq!(colors.dock_background, 0x2C2C2C);
        assert_eq!(colors.group_background, 0x252525);
        assert_eq!(colors.border, 0x383838);
        assert_eq!(colors.splitter, 0x444444);
        assert_eq!(colors.primary_text, 0xFFFFFF);
        assert_eq!(colors.muted_text, 0xB3B3B3);
        assert_eq!(colors.active_tab, 0x383838);
        assert_eq!(colors.hover, 0x383838);
        assert_eq!(colors.focus, 0x0C8CE9);
        assert_eq!(colors.drop_target, 0x0C8CE9);
        assert_eq!(colors.toolbar_background, 0x2C2C2C);
    }
}
