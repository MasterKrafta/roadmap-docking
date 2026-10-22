use crate::{NodeId, PanelId, Rect, ResizeEdge};

/// Geometry of the currently visible panel flyout.
///
/// The resize edge always faces the Canvas: right for a left-side rail and left
/// for a right-side rail. The flyout overlays the Canvas and does not reflow it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FlyoutRect {
    pub column: NodeId,
    pub group: NodeId,
    pub panel: PanelId,
    pub rect: Rect,
    pub resize_edge: ResizeEdge,
}

impl FlyoutRect {
    pub const fn new(
        column: NodeId,
        group: NodeId,
        panel: PanelId,
        rect: Rect,
        resize_edge: ResizeEdge,
    ) -> Self {
        Self {
            column,
            group,
            panel,
            rect,
            resize_edge,
        }
    }
}
