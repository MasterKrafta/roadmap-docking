/// A rejected drag-session operation.
///
/// Rejected operations do not change the current state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DragSessionError {
    #[error("a drag session can only begin while idle; reset a terminal state first")]
    SessionNotIdle,
    #[error("there is no active pointer gesture")]
    NoActiveGesture,
    #[error("a drop target can only be assigned while dragging")]
    NotDragging,
    #[error("the drag handle does not match the payload kind")]
    PayloadHandleMismatch,
    #[error("a floating title drag moves the window and cannot accept a dock target")]
    NotDockingDrag,
    #[error("an active gesture cannot be reset before it is dropped or cancelled")]
    CannotResetActiveGesture,
    #[error("pointer coordinates must be finite")]
    InvalidPointerPosition,
    #[error("drag threshold must be finite and greater than zero")]
    InvalidThreshold,
}
