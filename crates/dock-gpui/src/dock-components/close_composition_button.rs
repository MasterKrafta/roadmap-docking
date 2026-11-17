use gpui::{
    App, IntoElement, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, div,
    px, rgb,
};
use ui::Theme;

pub(crate) type CloseCompositionAction = Box<dyn Fn(&mut App) + 'static>;

/// Visual close control for the entire composition.
pub struct CloseCompositionButton;

impl CloseCompositionButton {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        Self::render_with_action(id, theme, 32.0, 14.0, Box::new(|_| {}))
    }

    pub fn render_with_metrics(
        id: String,
        theme: Theme,
        width: f32,
        icon_size: f32,
    ) -> impl IntoElement {
        Self::render_with_action(id, theme, width, icon_size, Box::new(|_| {}))
    }

    pub(crate) fn render_with_action(
        id: String,
        theme: Theme,
        width: f32,
        icon_size: f32,
        on_click: CloseCompositionAction,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w(px(width))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.icon_button_background))
            .border_1()
            .border_color(rgb(theme.colors.icon_button_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(icon_size))
            .child("×")
            .on_click(move |_, _, app| {
                on_click(app);
                app.stop_propagation();
            })
    }
}
