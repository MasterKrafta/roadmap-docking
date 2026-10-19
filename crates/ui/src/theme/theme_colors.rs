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
    pub toolbar_background: Rgb,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            window_background: 0x202124,
            canvas_background: 0x17181A,
            dock_background: 0x2B2D31,
            group_background: 0x25272B,
            border: 0x41444A,
            splitter: 0x50545B,
            primary_text: 0xE4E6EB,
            muted_text: 0xA5A9B2,
            active_tab: 0x3A3D43,
            hover: 0x383B41,
            focus: 0x7AA2F7,
            toolbar_background: 0x303238,
        }
    }
}
