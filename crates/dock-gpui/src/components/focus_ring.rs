use gpui::{StatefulInteractiveElement, Styled, rgb};
use ui::Theme;

/// Makes a stateful GPUI element keyboard-focusable and displays the shared
/// visible focus outline.
pub fn with_focus_ring<E>(element: E, theme: &Theme) -> E
where
    E: StatefulInteractiveElement,
{
    let focus_color = theme.colors.focus;
    element
        .focusable()
        .focus_visible(move |style| style.border_2().border_color(rgb(focus_color)))
}
