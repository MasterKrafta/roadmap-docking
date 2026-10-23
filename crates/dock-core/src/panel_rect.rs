use crate::{PanelId, Rect};

/// Geometry of a panel's content region in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelRect {
    pub panel: PanelId,
    pub rect: Rect,
}

impl PanelRect {
    pub const fn new(panel: PanelId, rect: Rect) -> Self {
        Self { panel, rect }
    }
}
