use dock_core::Rect;
use gpui::{AnyElement, App, IntoElement, RenderOnce, Window};
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
        RectBox::new(self.rect, self.theme.colors.group_background).child(self.content)
    }
}
