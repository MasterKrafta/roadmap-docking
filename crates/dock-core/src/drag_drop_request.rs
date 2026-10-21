use crate::{DragPayload, DropTarget, Point};

/// The single semantic drop intent emitted by a successful drag release.
///
/// This is not a command and does not mutate Layout. `payload` preserves the source
/// semantics: Panel means one tab, Group means the complete group-grip payload,
/// and FloatingGroup means the window itself. The adapter translates this one
/// request into exactly one `DockCommand` and applies it once. Targets such as
/// `DropTarget::FreeArea` may require the adapter to supply data such as floating
/// bounds when forming that command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragDropRequest {
    pub payload: DragPayload,
    pub origin: Point,
    pub point: Point,
    pub target: DropTarget,
}
