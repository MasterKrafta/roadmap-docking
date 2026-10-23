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
    pub const fn default_for(payload: crate::DragPayload) -> Self {
        match payload {
            crate::DragPayload::Panel { .. } => Self::PanelTab,
            crate::DragPayload::Group { .. } => Self::GroupGrip,
            crate::DragPayload::FloatingGroup { .. } => Self::FloatingTitle,
        }
    }

    pub const fn accepts(self, payload: crate::DragPayload) -> bool {
        matches!(
            (self, payload),
            (Self::PanelTab, crate::DragPayload::Panel { .. })
                | (Self::FlyoutPanel, crate::DragPayload::Panel { .. })
                | (Self::GroupGrip, crate::DragPayload::Group { .. })
                | (
                    Self::FloatingTitle,
                    crate::DragPayload::FloatingGroup { .. }
                )
                | (
                    Self::FloatingCapture,
                    crate::DragPayload::FloatingGroup { .. }
                )
        )
    }
}
