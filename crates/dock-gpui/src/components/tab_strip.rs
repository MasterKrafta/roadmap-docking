use dock_core::Rect;
use gpui::{App, IntoElement, RenderOnce, Window};
use ui::Theme;

use crate::components::RectBox;

/// The toolbar-colored background behind a group's tab headers and controls.
#[derive(IntoElement)]
pub struct TabStrip {
    /// Absolute bounds supplied by Core's frame geometry.
    pub rect: Rect,
    /// Shared application theme.
    pub theme: Theme,
}

impl RenderOnce for TabStrip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        RectBox::new(self.rect, self.theme.colors.tab_strip_background)
    }
}
