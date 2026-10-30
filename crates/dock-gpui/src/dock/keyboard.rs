//! Keyboard shortcut translation kept independent from window event dispatch.

use gpui::Keystroke;

/// Window-level shortcut actions. Menu navigation and Escape are handled by the
/// active transient interaction before these global shortcuts are considered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shortcut {
    Undo,
    Redo,
    ToggleChrome,
    ToggleDockChrome,
}

/// Resolves dock shortcuts from GPUI's platform-normalized keystroke metadata.
///
/// The caller installs this handler in the bubble phase so focused input controls
/// can consume their own Tab and Undo/Redo keystrokes first.
pub(crate) fn resolve(keystroke: &Keystroke, menu_open: bool) -> Option<Shortcut> {
    if menu_open {
        return None;
    }

    let key = keystroke.key.to_ascii_lowercase();
    let modifiers = keystroke.modifiers;
    let command_modifier = modifiers.control || modifiers.platform;

    if command_modifier && !modifiers.alt && !modifiers.function {
        return match (key.as_str(), modifiers.shift) {
            ("z", true) => Some(Shortcut::Redo),
            ("z", false) => Some(Shortcut::Undo),
            ("y", false) => Some(Shortcut::Redo),
            _ => None,
        };
    }

    if key == "tab"
        && !modifiers.control
        && !modifiers.platform
        && !modifiers.alt
        && !modifiers.function
    {
        return Some(if modifiers.shift {
            Shortcut::ToggleDockChrome
        } else {
            Shortcut::ToggleChrome
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{Shortcut, resolve};
    use gpui::{Keystroke, Modifiers};

    fn key(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            key: key.to_owned(),
            modifiers,
            key_char: None,
        }
    }

    #[test]
    fn resolves_undo_redo_and_tab_visibility_cycle() {
        assert_eq!(
            resolve(
                &key(
                    "z",
                    Modifiers {
                        control: true,
                        ..Modifiers::default()
                    }
                ),
                false
            ),
            Some(Shortcut::Undo)
        );
        assert_eq!(
            resolve(
                &key(
                    "z",
                    Modifiers {
                        control: true,
                        shift: true,
                        ..Modifiers::default()
                    }
                ),
                false
            ),
            Some(Shortcut::Redo)
        );
        assert_eq!(
            resolve(
                &key(
                    "y",
                    Modifiers {
                        control: true,
                        ..Modifiers::default()
                    }
                ),
                false
            ),
            Some(Shortcut::Redo)
        );
        assert_eq!(
            resolve(&key("tab", Modifiers::default()), false),
            Some(Shortcut::ToggleChrome)
        );
        assert_eq!(
            resolve(
                &key(
                    "tab",
                    Modifiers {
                        shift: true,
                        ..Modifiers::default()
                    }
                ),
                false
            ),
            Some(Shortcut::ToggleDockChrome)
        );
    }

    #[test]
    fn shortcuts_do_not_override_an_open_menu_or_alt_modified_input() {
        assert_eq!(
            resolve(
                &key(
                    "z",
                    Modifiers {
                        control: true,
                        ..Modifiers::default()
                    }
                ),
                true
            ),
            None
        );
        assert_eq!(
            resolve(
                &key(
                    "tab",
                    Modifiers {
                        alt: true,
                        ..Modifiers::default()
                    }
                ),
                false
            ),
            None
        );
    }
}
