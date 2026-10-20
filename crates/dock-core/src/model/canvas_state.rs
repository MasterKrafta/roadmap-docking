use crate::geometry::Rect;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CanvasState {
    pub bounds: Rect,
}
