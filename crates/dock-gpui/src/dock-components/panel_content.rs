use gpui::{AnyView, InteractiveElement, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Draws the visual surface that owns a panel's actual content.
pub struct PanelContent;

impl PanelContent {
    /// Renders an empty surface for static previews.
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        Self::render_with_view(id, theme, None)
    }

    /// Renders the panel surface and mounts its live view when one is available.
    ///
    /// The view can be temporarily absent during a host transfer. The surface
    /// remains painted by this component while the cached AnyView is in transit.
    pub(crate) fn render_with_view(
        id: String,
        theme: Theme,
        view: Option<AnyView>,
    ) -> impl IntoElement {
        let outline_id = format!("{id}-debug-outline");
        let mut surface = div()
            .id(id)
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme.colors.panel_surface_background))
            .text_color(rgb(theme.colors.primary_text));

        if let Some(view) = view {
            surface = surface.child(view);
        }

        // TEMP: Inset outline shows the actual PanelContent bounds separately
        // from PanelHost's green PanelRect outline.
        surface.child(
            div()
                .id(outline_id)
                .absolute()
                .top(px(3.0))
                .left(px(3.0))
                .right(px(3.0))
                .bottom(px(3.0))
                .border_2()
                .border_color(rgb(0x00BFFF)),
        )
    }
}
