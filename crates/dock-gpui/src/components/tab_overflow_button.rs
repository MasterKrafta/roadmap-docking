use dock_core::{NodeId, Rect};
use gpui::{App, ClickEvent, IntoElement, RenderOnce, Window};
use ui::Theme;

use crate::components::IconButton;

/// The button that opens the list of tabs hidden by group overflow.
#[derive(IntoElement)]
pub struct TabOverflowButton {
    /// Tabs-group identity for a stable element key.
    pub group: NodeId,
    /// Button geometry calculated for the current frame.
    pub rect: Rect,
    /// Shared application theme.
    pub theme: Theme,
    /// Opens the overflow menu.
    pub on_open: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
}

impl RenderOnce for TabOverflowButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        IconButton::new(
            format!("dock-tab-overflow-{:?}", self.group),
            self.rect,
            "⋯",
            &self.theme,
            self.on_open,
        )
    }
}
