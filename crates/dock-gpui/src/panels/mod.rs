//! Central registry of panel metadata and content renderers.

use dock_core::{PanelConstraints, PanelInstance, PanelKey, Size};
use gpui::SharedString;
use ui::Theme;

use gpui::AnyElement;

pub mod content;

/// Describes one supported panel kind without owning its runtime layout state.
pub struct PanelDef {
    pub key: &'static str,
    pub title: SharedString,
    pub icon: &'static str,
    pub constraints: fn(&Theme) -> PanelConstraints,
    pub content: fn(&PanelInstance) -> AnyElement,
}

/// Built-in panels in the order used by the panel menu and initial layout.
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
        let keys = PANELS.iter().map(|definition| definition.key).collect::<Vec<_>>();
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
        assert!(values.iter().all(|(_, constraints)| constraints.validate().is_ok()));
    }

    #[test]
    fn def_for_returns_none_for_an_unknown_panel_kind() {
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        let unknown = PanelInstance::new(
            PanelKey::new("unknown-panel").expect("test key should be valid"),
            constraints,
        );

        assert!(def_for(&unknown).is_none());
    }
}
