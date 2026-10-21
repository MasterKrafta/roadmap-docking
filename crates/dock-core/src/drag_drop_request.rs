use crate::{DragPayload, DropTarget, Point};

/// The single semantic drop intent emitted by a successful drag release.
///
/// This is not a command and does not mutate Layout. The adapter translates
/// this one request into exactly one `DockCommand` and applies it once. Targets
/// such as `DropTarget::FreeArea` may require the adapter to supply data such as
/// floating bounds when forming that command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragDropRequest {
    pub payload: DragPayload,
    pub origin: Point,
    pub point: Point,
    pub target: DropTarget,
}
