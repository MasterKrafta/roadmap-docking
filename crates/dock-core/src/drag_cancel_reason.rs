/// Why a drag gesture ended without a drop request.
///
/// The adapter maps Escape and focus loss to these core reasons; no UI-specific
/// key or window type is required by dock-core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragCancelReason {
    /// Pointer was released before movement crossed the drag threshold.
    BelowThreshold,
    /// The user cancelled the gesture with Escape.
    Escape,
    /// The input target lost focus while the gesture was active.
    FocusLost,
    /// Pointer was released without a valid resolved drop target.
    NoValidTarget,
    /// An explicit adapter/application cancellation.
    Explicit,
    /// A non-finite pointer coordinate invalidated the active gesture.
    InvalidPointerPosition,
}
