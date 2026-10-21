use crate::{DropTarget, DropZone, Rect};

/// Geometry of a drop affordance paired with its semantic destination.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DropZoneRect {
    pub target: DropTarget,
    pub zone: DropZone,
    pub rect: Rect,
}

impl DropZoneRect {
    pub const fn new(target: DropTarget, zone: DropZone, rect: Rect) -> Self {
        Self { target, zone, rect }
    }
}
