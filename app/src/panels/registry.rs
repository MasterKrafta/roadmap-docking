use dock_core::{DockCommand, PanelConstraints, PanelInstance, PanelKey, Size};
use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

type PanelFactory = fn(&Theme) -> AnyElement;
type PanelMenuFactory = fn(&PanelInstance) -> Vec<PanelMenuItem>;
type PanelEntry = (
    PanelKey,
    &'static str,
    PanelConstraints,
    PanelFactory,
    PanelMenuFactory,
);

/// A panel-owned menu contribution. Commands are intents; MainWindow applies them through Core.
#[derive(Clone)]
pub(crate) struct PanelMenuItem {
    pub(crate) label: &'static str,
    pub(crate) command: DockCommand,
}

/// App-owned panel descriptors, view factories, and panel menu sources.
///
/// Dock Core receives only each stable key and its size constraints. GPUI trees
/// are created lazily by render, which the adapter calls for visible active panels.
pub(crate) struct PanelRegistry {
    entries: Vec<PanelEntry>,
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
                    no_panel_menu,
                ),
                (
                    PanelKey::new("color").expect("valid built-in key"),
                    "Color",
                    constraints,
                    render_color,
                    no_panel_menu,
                ),
                (
                    PanelKey::new("properties").expect("valid built-in key"),
                    "Properties",
                    constraints,
                    render_properties,
                    no_panel_menu,
                ),
                (
                    PanelKey::new("history").expect("valid built-in key"),
                    "History",
                    constraints,
                    render_history,
                    no_panel_menu,
                ),
            ],
        }
    }
}

impl PanelRegistry {
    pub(crate) fn core_descriptors(&self) -> Vec<(PanelKey, PanelConstraints)> {
        self.entries
            .iter()
            .map(|(key, _, constraints, _, _)| (key.clone(), *constraints))
            .collect()
    }

    pub(crate) fn menu_descriptors(&self) -> Vec<(PanelKey, String)> {
        self.entries
            .iter()
            .map(|(key, title, _, _, _)| (key.clone(), (*title).to_owned()))
            .collect()
    }

    pub(crate) fn constraints_for_key(&self, wanted: &PanelKey) -> Option<PanelConstraints> {
        self.entries
            .iter()
            .find(|(key, _, _, _, _)| key.as_str() == wanted.as_str())
            .map(|(_, _, constraints, _, _)| *constraints)
    }

    pub(crate) fn title(&self, instance: &PanelInstance) -> String {
        self.entries
            .iter()
            .find(|(key, _, _, _, _)| key.as_str() == instance.key.as_str())
            .map(|(_, title, _, _, _)| (*title).to_owned())
            .unwrap_or_else(|| format!("Unavailable: {}", instance.key.as_str()))
    }

    pub(crate) fn icon(&self, instance: &PanelInstance) -> &'static str {
        panel_icon_for_key(instance.key.as_str())
    }

    pub(crate) fn menu_items(&self, instance: &PanelInstance) -> Vec<PanelMenuItem> {
        self.entries
            .iter()
            .find(|(key, _, _, _, _)| key.as_str() == instance.key.as_str())
            .map(|(_, _, _, _, menu_factory)| menu_factory(instance))
            .unwrap_or_default()
    }

    pub(crate) fn render(&self, instance: &PanelInstance, theme: &Theme) -> AnyElement {
        self.entries
            .iter()
            .find(|(key, _, _, _, _)| key.as_str() == instance.key.as_str())
            .map(|(_, _, _, factory, _)| factory(theme))
            .unwrap_or_else(|| {
                panel_content(theme, "Unavailable panel", instance.key.as_str())
            })
    }
}

fn panel_icon_for_key(key: &str) -> &'static str {
    match key {
        "layers" => "▱",
        "color" => "◉",
        "properties" => "☷",
        "history" => "↶",
        _ => "◇",
    }
}

fn no_panel_menu(_instance: &PanelInstance) -> Vec<PanelMenuItem> {
    // Built-in panels are placeholders and do not yet expose panel-specific actions.
    Vec::new()
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

#[cfg(test)]
mod tests {
    use super::panel_icon_for_key;

    #[test]
    fn panel_icons_are_keyed_to_panel_identity_not_title_initials() {
        assert_eq!(panel_icon_for_key("layers"), "▱");
        assert_eq!(panel_icon_for_key("color"), "◉");
        assert_eq!(panel_icon_for_key("properties"), "☷");
        assert_eq!(panel_icon_for_key("history"), "↶");
        assert_eq!(panel_icon_for_key("unregistered"), "◇");
    }
}
