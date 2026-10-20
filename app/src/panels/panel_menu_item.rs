/// A panel-specific menu action contributed by that panel's descriptor.
///
/// Shared dock commands such as Close and Close Tab Group are composed by the dock UI,
/// not by this type or a panel-specific menu source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelMenuItem {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub separator_before: bool,
}

impl PanelMenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            separator_before: false,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn with_separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
}
