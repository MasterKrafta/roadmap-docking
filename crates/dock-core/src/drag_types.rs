//! Semantic types and outcomes used by the drag interaction state machine.

use crate::{DockCommand, DropTarget, FloatingGroupId, NodeId, PanelId, Point, Rect};

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

/// A pure result of resolving a released drag request against a Layout.
///
/// `NoOp` means the source was dropped onto itself or its existing position;
/// callers must not submit a command for it. `Command` contains exactly one
/// command for the adapter to execute once.
#[derive(Clone, Debug, PartialEq)]
pub enum DragDropAction {
    NoOp,
    Command(DockCommand),
}

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
    /// Which semantic surface initiated the gesture (tab, flyout panel, group grip, title, or capture zone).
    pub handle: DragHandle,
    pub origin: Point,
    pub point: Point,
    /// Absent for FloatingTitle: releasing a title drag moves the window and needs no dock target.
    pub target: Option<DropTarget>,
    /// Optional bounds supplied by the adapter for a FreeArea/Float request.
    /// DragSession leaves this empty because it does not own render geometry.
    pub floating_bounds: Option<Rect>,
}

/// The interaction surface that initiated a drag.
///
/// UI adapters map their hit result to this semantic value. Dock Core does not
/// receive GPUI pointer events and does not infer title-bar movement from a
/// DropTarget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragHandle {
    /// One tab header; moves/reorders a single Panel.
    PanelTab,
    /// A panel surface in a rail flyout; the source flyout remains visible during the gesture.
    ///
    /// A completed valid drop closes that source flyout; cancelled gestures leave it open.
    FlyoutPanel,
    /// A group's grip/header; moves the whole Tabs node.
    GroupGrip,
    /// A floating window title bar; moves window bounds, never docks it.
    FloatingTitle,
    /// An explicit floating-window capture affordance; may dock/stack the
    /// entire floating root using the resolved DropTarget.
    FloatingCapture,
}

impl DragHandle {
    pub const fn default_for(payload: DragPayload) -> Self {
        match payload {
            DragPayload::Panel { .. } => Self::PanelTab,
            DragPayload::Group { .. } => Self::GroupGrip,
            DragPayload::FloatingGroup { .. } => Self::FloatingTitle,
        }
    }

    pub const fn accepts(self, payload: DragPayload) -> bool {
        matches!(
            (self, payload),
            (Self::PanelTab, DragPayload::Panel { .. })
                | (Self::FlyoutPanel, DragPayload::Panel { .. })
                | (Self::GroupGrip, DragPayload::Group { .. })
                | (
                    Self::FloatingTitle,
                    DragPayload::FloatingGroup { .. }
                )
                | (
                    Self::FloatingCapture,
                    DragPayload::FloatingGroup { .. }
                )
        )
    }
}

/// The semantic model object carried by a drag session.
///
/// These identifiers are runtime handles. The command/layout layer must
/// revalidate them when a drop is applied; this transient payload is not a
/// persistent identity or a second copy of the layout tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragPayload {
    /// One panel tab. The request carries only this panel's identity.
    Panel { panel: PanelId },
    /// A whole Tabs group being dragged from its group grip.
    ///
    /// Keep this variant distinct from Panel: every child tab moves together,
    /// and the existing Tabs node retains active_index and collapsed state when
    /// it is moved, split, or floated as a group.
    Group { group: NodeId },
    /// A floating window moved by its window/title drag surface.
    ///
    /// This is distinct from dragging the Tabs group inside that window.
    FloatingGroup { group: FloatingGroupId },
}

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

/// State of one UI-independent pointer drag gesture.
///
/// All positions are in logical pixels. This is transient interaction state,
/// not part of the persistent layout model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragState {
    /// No gesture is active.
    Idle,
    /// Pointer is down, but movement has not crossed the drag threshold.
    Pressed {
        payload: DragPayload,
        origin: Point,
        position: Point,
    },
    /// Movement crossed the threshold; target is refreshed by the resolver
    /// after pointer movement. The Layout itself remains untouched.
    Dragging {
        payload: DragPayload,
        origin: Point,
        position: Point,
        target: Option<DropTarget>,
    },
    /// A valid target was present when the pointer was released.
    Dropped {
        payload: DragPayload,
        origin: Point,
        position: Point,
        target: Option<DropTarget>,
    },
    /// Gesture ended without a drop request.
    Cancelled {
        payload: DragPayload,
        origin: Point,
        position: Point,
        reason: DragCancelReason,
    },
}
