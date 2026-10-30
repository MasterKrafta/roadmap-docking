use dock_core::{NodeId, Rect};
use gpui::{App, IntoElement, RenderOnce, Window};
use ui::Theme;

use crate::components::{ClickHandler, IconButton};

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
    pub on_open: ClickHandler,
}

impl RenderOnce for TabOverflowButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let background_color = self.theme.colors.tab_overflow_button;
        let hover_color = self.theme.colors.tab_overflow_button_hover;
        IconButton::new(
            format!("dock-tab-overflow-{:?}", self.group),
            self.rect,
            "⋯",
            &self.theme,
            self.on_open,
        )
        .with_colors(background_color, hover_color)
    }
}
