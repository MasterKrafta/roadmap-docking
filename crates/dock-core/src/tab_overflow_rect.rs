use crate::{NodeId, PanelId, Rect};

/// Geometry and semantic contents of a tab-strip overflow button.
///
/// This overflow menu is an implementation-specific UX choice, not a claim
/// about Photoshop's native tab-strip behavior. Adapters can use
/// `hidden_tabs` to populate a menu and submit `DockCommand::ActivateTab`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TabOverflowRect {
    pub group: NodeId,
    pub button_rect: Rect,
    pub hidden_tabs: Vec<PanelId>,
}

impl TabOverflowRect {
    pub fn new(group: NodeId, button_rect: Rect, hidden_tabs: Vec<PanelId>) -> Self {
        Self {
            group,
            button_rect,
            hidden_tabs,
        }
    }
}
