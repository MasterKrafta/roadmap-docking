use crate::geometry::Size;

/// Layout-only panel metadata. Scroll position, selection inside panel content, and other
/// content state belong to the panel implementation and are intentionally not stored here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelState {
    last_known_size: Option<Size>,
}

impl PanelState {
    pub const fn new(last_known_size: Option<Size>) -> Self { Self { last_known_size } }
    pub const fn last_known_size(&self) -> Option<Size> { self.last_known_size }
}
