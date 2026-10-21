use gpui::{Context, IntoElement, Render, Window, div, rgb};
use ui::Theme;

/// The initial application window. Its theme is retained for subsequent UI construction.
pub(crate) struct StartupWindow {
    _theme: Theme,
}

impl Render for StartupWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        use gpui::ParentElement;
        use gpui::Styled;

        div()
            .max_size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x252526))
            .text_color(rgb(0xe0e0e0))
            .child("Docking — Core")
    }
}
