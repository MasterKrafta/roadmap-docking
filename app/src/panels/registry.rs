use dock_core::{PanelConstraints, PanelInstance, PanelKey, Size};
use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// App-owned panel descriptors and content factories.
///
/// Dock Core receives only each stable key and its size constraints. GPUI trees
/// are created lazily by `render`, which the adapter calls for visible active panels.
pub(crate) struct PanelRegistry {
    entries: Vec<(PanelKey, &'static str, PanelConstraints, fn(&Theme) -> AnyElement)>,
}

impl Default for PanelRegistry {
    fn default() -> Self {
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        Self {
            entries: vec![
                (
                    PanelKey::new("layers").expect("valid built-in key"),
                    "Layers",
                    constraints,
                    render_layers,
                ),
                (
                    PanelKey::new("color").expect("valid built-in key"),
                    "Color",
                    constraints,
                    render_color,
                ),
                (
                    PanelKey::new("properties").expect("valid built-in key"),
                    "Properties",
                    constraints,
                    render_properties,
                ),
                (
                    PanelKey::new("history").expect("valid built-in key"),
                    "History",
                    constraints,
                    render_history,
                ),
            ],
        }
    }
}

impl PanelRegistry {
    pub(crate) fn core_descriptors(&self) -> Vec<(PanelKey, PanelConstraints)> {
        self.entries
            .iter()
            .map(|(key, _, constraints, _)| (key.clone(), *constraints))
            .collect()
    }

    pub(crate) fn title(&self, instance: &PanelInstance) -> String {
        self.entries
            .iter()
            .find(|(key, _, _, _)| key.as_str() == instance.key.as_str())
            .map(|(_, title, _, _)| (*title).to_owned())
            .unwrap_or_else(|| format!("Unavailable: {}", instance.key.as_str()))
    }

    pub(crate) fn render(&self, instance: &PanelInstance, theme: &Theme) -> AnyElement {
        self.entries
            .iter()
            .find(|(key, _, _, _)| key.as_str() == instance.key.as_str())
            .map(|(_, _, _, factory)| factory(theme))
            .unwrap_or_else(|| {
                panel_content(theme, "Unavailable panel", instance.key.as_str())
            })
    }
}

fn panel_content(theme: &Theme, title: &str, description: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p(px(theme.metrics.content_padding))
        .text_color(rgb(theme.colors.primary_text))
        .child(
            div()
                .text_size(px(theme.typography.title_size))
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_color(rgb(theme.colors.muted_text))
                .child(description.to_owned()),
        )
        .into_any_element()
}

fn render_layers(theme: &Theme) -> AnyElement {
    panel_content(theme, "Layers", "Layer stack and visibility controls")
}

fn render_color(theme: &Theme) -> AnyElement {
    panel_content(theme, "Color", "Foreground, background, and color controls")
}

fn render_properties(theme: &Theme) -> AnyElement {
    panel_content(theme, "Properties", "Properties for the current selection")
}

fn render_history(theme: &Theme) -> AnyElement {
    panel_content(theme, "History", "Recent document operations")
}
