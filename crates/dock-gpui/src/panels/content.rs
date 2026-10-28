use dock_core::PanelInstance;
use gpui::{AnyElement, Div, IntoElement, ParentElement, Styled, div, px, rgb};

use crate::labels::UNKNOWN_PANEL_TITLE;

/// Renders intentionally generic placeholder content for the bundled reference window.
///
/// This is the sample registry callback, not an application data renderer. The enclosing dock
/// element owns the bounds and background; embedded applications should return their own inner
/// content from [`crate::host::DockHost::render_panel`].
pub fn render_registered(instance: &PanelInstance) -> AnyElement {
    let selected_tab = super::panel_title(instance);

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .child(format!("Контент: {selected_tab}"))
        .child(div().child(format!("Выбрана {selected_tab}")))
        .into_any_element()
}

/// Renders a visible fallback when Core contains a panel key that has no registry entry.
pub fn render_missing(instance: &PanelInstance) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .child(format!("{UNKNOWN_PANEL_TITLE}: {}", instance.key))
        .into_any_element()
}

/// Renders the legacy sized placeholder while the app window is being migrated.
pub fn render(
    theme: ui::Theme,
    selected_tab: &'static str,
    width: f32,
    height: f32,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.group_background))
        .text_color(rgb(theme.colors.primary_text))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(div().child(format!("Контент: {selected_tab}")))
        .child(
            div()
                .text_color(rgb(theme.colors.muted_text))
                .child(format!("Выбрана {selected_tab}")),
        )
}
