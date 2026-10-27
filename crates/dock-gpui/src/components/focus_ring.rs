use gpui::{Keystroke, StatefulInteractiveElement, Styled, rgb};
use ui::Theme;

/// Direction requested when reordering a focused tab with the keyboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TabReorderDirection {
    Previous,
    Next,
}

/// Recognizes the platform command modifier + Shift + Left/Right tab-reorder shortcut.
pub(crate) fn keyboard_tab_reorder_direction(
    keystroke: &Keystroke,
) -> Option<TabReorderDirection> {
    let modifiers = keystroke.modifiers;
    if !(modifiers.control || modifiers.platform)
        || !modifiers.shift
        || modifiers.alt
        || modifiers.function
    {
        return None;
    }

    match keystroke.key.to_ascii_lowercase().as_str() {
        "left" | "arrowleft" => Some(TabReorderDirection::Previous),
        "right" | "arrowright" => Some(TabReorderDirection::Next),
        _ => None,
    }
}

/// Returns whether a keystroke should activate a focused dock control.
pub(crate) fn is_keyboard_activation_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("enter")
        || key.eq_ignore_ascii_case("return")
        || key.eq_ignore_ascii_case("space")
        || key == " "
}

/// Makes a stateful GPUI element keyboard-focusable and gives it the standard
/// visible focus outline used across the dock chrome.
pub fn with_focus_ring<E>(element: E, theme: &Theme) -> E
where
    E: StatefulInteractiveElement,
{
    let focus_color = theme.colors.focus;
    element
        .focusable()
        .focus_visible(move |style| style.border_2().border_color(rgb(focus_color)))
}

#[cfg(test)]
mod tests {
    use super::{
        TabReorderDirection, is_keyboard_activation_key, keyboard_tab_reorder_direction,
    };
    use gpui::{Keystroke, Modifiers};

    fn key(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            key: key.to_owned(),
            modifiers,
            key_char: None,
        }
    }

    #[test]
    fn keyboard_reorder_accepts_command_shift_arrows() {
        let control_shift = Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_tab_reorder_direction(&key("left", control_shift)),
            Some(TabReorderDirection::Previous)
        );
        assert_eq!(
            keyboard_tab_reorder_direction(&key("ArrowRight", control_shift)),
            Some(TabReorderDirection::Next)
        );

        let platform_shift = Modifiers {
            platform: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_tab_reorder_direction(&key("arrowleft", platform_shift)),
            Some(TabReorderDirection::Previous)
        );
    }

    #[test]
    fn keyboard_reorder_requires_modifiers_and_ignores_other_keys() {
        let no_modifiers = Modifiers::default();
        assert_eq!(keyboard_tab_reorder_direction(&key("left", no_modifiers)), None);

        let shift_only = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(keyboard_tab_reorder_direction(&key("right", shift_only)), None);

        let alt_modified = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(keyboard_tab_reorder_direction(&key("left", alt_modified)), None);

        let function_modified = Modifiers {
            control: true,
            shift: true,
            function: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_tab_reorder_direction(&key("right", function_modified)),
            None
        );
        assert_eq!(
            keyboard_tab_reorder_direction(&key("escape", Modifiers::default())),
            None
        );
    }

    #[test]
    fn keyboard_activation_accepts_enter_return_and_space() {
        assert!(is_keyboard_activation_key("enter"));
        assert!(is_keyboard_activation_key("Enter"));
        assert!(is_keyboard_activation_key("return"));
        assert!(is_keyboard_activation_key("space"));
        assert!(is_keyboard_activation_key(" "));
    }

    #[test]
    fn keyboard_activation_ignores_navigation_and_escape_keys() {
        assert!(!is_keyboard_activation_key("tab"));
        assert!(!is_keyboard_activation_key("left"));
        assert!(!is_keyboard_activation_key("escape"));
    }
}
