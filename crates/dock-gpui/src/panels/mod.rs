//! Built-in panel metadata and placeholder content registry used by the bundled reference window.
//!
//! Consumers embedding the dock can own their panel registry and render domain content through
//! [`crate::host::DockHost`]; they do not need to extend this sample registry.

use dock_core::{PanelConstraints, PanelInstance, PanelKey, Size};
use gpui::{AnyElement, SharedString};
use ui::Theme;

use crate::labels::UNKNOWN_PANEL_TITLE;

pub mod content;

/// Describes one supported panel kind without owning its runtime layout state.
pub struct PanelDef {
    pub key: &'static str,
    pub title: SharedString,
    pub icon: &'static str,
    pub constraints: fn(&Theme) -> PanelConstraints,
    pub content: fn(&PanelInstance) -> AnyElement,
}

/// Built-in sample panels in the order used by the reference window's panel menu and initial layout.
pub const PANELS: &[PanelDef] = &[
    PanelDef {
        key: "layers",
        title: SharedString::new_static("Вкладка 1"),
        icon: "▤",
        constraints: panel_constraints,
        content: content::render_registered,
    },
    PanelDef {
        key: "color",
        title: SharedString::new_static("Вкладка 2"),
        icon: "◉",
        constraints: panel_constraints,
        content: content::render_registered,
    },
    PanelDef {
        key: "properties",
        title: SharedString::new_static("Вкладка 3"),
        icon: "⚙",
        constraints: panel_constraints,
        content: content::render_registered,
    },
    PanelDef {
        key: "history",
        title: SharedString::new_static("Вкладка 4"),
        icon: "↶",
        constraints: panel_constraints,
        content: content::render_registered,
    },
];

/// Returns metadata for a known panel kind, or `None` for an unknown key.
pub fn def_for(instance: &PanelInstance) -> Option<&'static PanelDef> {
    PANELS
        .iter()
        .find(|definition| definition.key == instance.key.as_str())
}

/// Returns the title used by tabs, floating titles and flyout headers.
///
/// Unknown panel kinds use a visible fallback instead of panicking.
pub fn panel_title(instance: &PanelInstance) -> String {
    def_for(instance)
        .map(|definition| definition.title.to_string())
        .unwrap_or_else(|| UNKNOWN_PANEL_TITLE.to_owned())
}

/// Returns the rail icon for a panel, or a question mark for unknown kinds.
pub fn panel_icon(instance: &PanelInstance) -> &'static str {
    def_for(instance)
        .map(|definition| definition.icon)
        .unwrap_or("?")
}

/// Renders the registered panel content, using an explicit fallback for unknown kinds.
pub fn render_panel(instance: &PanelInstance) -> AnyElement {
    match def_for(instance) {
        Some(definition) => (definition.content)(instance),
        None => content::render_missing(instance),
    }
}

/// Creates the descriptors consumed by `create_initial_layout`.
pub fn descriptors(theme: &Theme) -> Vec<(PanelKey, PanelConstraints)> {
    PANELS
        .iter()
        .map(|definition| {
            let key = PanelKey::new(definition.key)
                .expect("built-in panel registry keys must be valid PanelKey values");
            let constraints = (definition.constraints)(theme);
            (key, constraints)
        })
        .collect()
}

fn panel_constraints(theme: &Theme) -> PanelConstraints {
    PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(theme.metrics.default_column_width.max(120.0), 220.0),
        None,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{PANELS, def_for, descriptors};
    use dock_core::{PanelConstraints, PanelInstance, PanelKey, Size};
    use ui::Theme;

    #[test]
    fn registry_contains_the_four_builtin_panels_in_menu_order() {
        let keys = PANELS
            .iter()
            .map(|definition| definition.key)
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["layers", "color", "properties", "history"]);

        for definition in PANELS {
            assert!(!definition.title.as_ref().is_empty());
            assert!(!definition.icon.is_empty());
        }
    }

    #[test]
    fn descriptors_use_registry_keys_and_valid_constraints() {
        let theme = Theme::default();
        let values = descriptors(&theme);
        let keys = values
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>();

        assert_eq!(keys, vec!["layers", "color", "properties", "history"]);
        assert!(
            values
                .iter()
                .all(|(_, constraints)| constraints.validate().is_ok())
        );
    }

    #[test]
    fn def_for_returns_none_for_an_unknown_panel_kind() {
        let constraints =
            PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false);
        let unknown = PanelInstance::new(
            PanelKey::new("unknown-panel").expect("test key should be valid"),
            constraints,
        );

        assert!(def_for(&unknown).is_none());
    }

    #[test]
    fn registry_helpers_supply_title_icon_and_unknown_fallbacks() {
        let constraints =
            PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false);
        let layers = PanelInstance::new(
            PanelKey::new("layers").expect("built-in key should be valid"),
            constraints,
        );
        assert_eq!(super::panel_title(&layers), "Вкладка 1");
        assert_eq!(super::panel_icon(&layers), "▤");

        let unknown = PanelInstance::new(
            PanelKey::new("unknown-panel").expect("test key should be valid"),
            constraints,
        );
        assert_eq!(
            super::panel_title(&unknown),
            crate::labels::UNKNOWN_PANEL_TITLE
        );
        assert_eq!(super::panel_icon(&unknown), "?");
        let _fallback = super::render_panel(&unknown);
    }
}
