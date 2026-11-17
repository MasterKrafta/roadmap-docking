use gpui::{Context, IntoElement, Render, Window, div};

/// Small content view used by the built-in launcher. Applications that need
/// their own panels can construct DockController and DockHost directly.
pub(super) struct StarterPanel {
    title: &'static str,
}

impl Render for StarterPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_4().child(self.title)
    }
}
