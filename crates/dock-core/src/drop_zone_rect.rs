use crate::{DropTarget, DropZone, Rect};

/// Ephemeral overlay geometry paired with its authoritative semantic destination.
///
/// `target` contains stable model IDs and insertion indices; render adapters consume
/// these values directly instead of building a second copy of the layout tree.
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
