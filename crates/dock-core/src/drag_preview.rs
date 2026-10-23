use crate::{DragPayload, DropTarget, Point};

/// Ephemeral data for rendering an active drag without editing the Layout.
///
/// `target` is the most recently resolved target for `point`; it is absent
/// when the pointer is not over an actionable drop zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragPreview {
    pub payload: DragPayload,
    pub point: Point,
    pub target: Option<DropTarget>,
}
