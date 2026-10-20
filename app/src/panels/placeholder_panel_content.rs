use dock_core::PanelKey;

/// Framework-neutral content model used by the four built-in prototype panels.
/// The GPUI renderer can turn this into a real placeholder view in the window stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceholderPanelContent {
    pub panel_key: PanelKey,
    pub heading: String,
    pub message: String,
}

impl PlaceholderPanelContent {
    pub fn new(
        panel_key: PanelKey,
        heading: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            panel_key,
            heading: heading.into(),
            message: message.into(),
        }
    }
}
