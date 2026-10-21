use crate::PanelId;

/// A leaf in the layout tree that points to a unique panel instance.
///
/// The panel's stable kind is stored separately in crate::PanelInstance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Panel {
    pub panel_id: PanelId,
}

impl Panel {
    pub fn new(panel_id: PanelId) -> Self {
        Self { panel_id }
    }
}
