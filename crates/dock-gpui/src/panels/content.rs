use dock_core::PanelInstance;
use gpui::{AnyElement, Div, IntoElement, ParentElement, Styled, div, px, rgb};

/// Renders placeholder content for a panel selected by the Core layout.
///
/// This is the registry callback. The enclosing dock element owns its bounds
/// and background, so this function only supplies the panel's inner content.
pub fn render_registered(instance: &PanelInstance) -> AnyElement {
    let selected_tab = super::def_for(instance)
        .map(|definition| definition.title.to_string())
        .unwrap_or_else(|| "Missing panel".to_owned());

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .child(format!("Контент: {selected_tab}"))
        .child(div().child(format!("Выбрана {selected_tab}")))
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
