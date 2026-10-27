use gpui::{StatefulInteractiveElement, Styled, rgb};
use ui::Theme;

/// Makes a stateful GPUI element keyboard-focusable and gives it the standard
/// visible focus outline used across the dock chrome.
/// Returns whether a keystroke should activate a focused dock control.
pub(crate) fn is_keyboard_activation_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("enter")
        || key.eq_ignore_ascii_case("return")
        || key.eq_ignore_ascii_case("space")
        || key == " "
}

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
    use super::is_keyboard_activation_key;

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
