use dock_core::Rect;
use gpui::{AnyElement, App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use ui::Theme;

use crate::components::RectBox;

/// The visible content surface for an active panel.
#[derive(IntoElement)]
pub struct PanelSurface {
    /// Absolute bounds supplied by Core's frame geometry.
    pub rect: Rect,
    /// Shared application theme.
    pub theme: Theme,
    /// Content element produced by the application-owned panel factory.
    pub content: AnyElement,
}

impl RenderOnce for PanelSurface {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let content_width = self.rect.size.width;
        let content_height = self.rect.size.height;
        let padding = self.theme.metrics.content_padding;

        RectBox::new(self.rect, self.theme.colors.group_background).child(
            div()
                .w(px(content_width))
                .h(px(content_height))
                .p(px(padding))
                .child(self.content),
        )
    }
}
