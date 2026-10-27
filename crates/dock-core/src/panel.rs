use crate::Rect;
use slotmap::new_key_type;

new_key_type! {
    /// Runtime identifier for one panel instance.
    pub struct PanelId;
}

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

/// Geometry of a panel's content region in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelRect {
    pub panel: PanelId,
    pub rect: Rect,
}

impl PanelRect {
    pub const fn new(panel: PanelId, rect: Rect) -> Self {
        Self { panel, rect }
    }
}

#[cfg(test)]
mod panel_id_tests {
    use super::PanelId;
    use slotmap::SlotMap;

    #[test]
    fn removed_panel_id_no_longer_resolves() {
        let mut panels = SlotMap::<PanelId, &'static str>::with_key();
        let id = panels.insert("layers instance");
        assert_eq!(panels.get(id), Some(&"layers instance"));
        assert_eq!(panels.remove(id), Some("layers instance"));
        assert!(panels.get(id).is_none());
    }
}
