use crate::{DragHandle, DragPayload, DropTarget, Point, Rect};

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
    /// Which semantic surface initiated the gesture (tab, group grip, title, or capture zone).
    pub handle: DragHandle,
    pub origin: Point,
    pub point: Point,
    /// Absent for FloatingTitle: releasing a title drag moves the window and needs no dock target.
    pub target: Option<DropTarget>,
    /// Optional bounds supplied by the adapter for a FreeArea/Float request.
    /// DragSession leaves this empty because it does not own render geometry.
    pub floating_bounds: Option<Rect>,
}
