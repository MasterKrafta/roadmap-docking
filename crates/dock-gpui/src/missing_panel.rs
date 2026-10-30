use dock_core::PanelKey;
use gpui::{Context, IntoElement, Render, Window, div};

/// Neutral fallback shown when a workspace contains a panel key that the
/// current application build does not register.
pub struct MissingPanelPlaceholder {
    key: PanelKey,
}

impl MissingPanelPlaceholder {
    pub fn new(key: PanelKey) -> Self {
        Self { key }
    }

    /// Stable application-level key of the panel that could not be resolved.
    pub fn key(&self) -> &PanelKey {
        &self.key
    }
}

impl Render for MissingPanelPlaceholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(format!("Missing panel: {}", self.key.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use dock_core::PanelKey;

    use super::MissingPanelPlaceholder;

    #[test]
    fn placeholder_keeps_the_unknown_panel_key() {
        let key = PanelKey::new("legacy-tool").unwrap();
        let placeholder = MissingPanelPlaceholder::new(key.clone());

        assert_eq!(placeholder.key(), &key);
    }
}
