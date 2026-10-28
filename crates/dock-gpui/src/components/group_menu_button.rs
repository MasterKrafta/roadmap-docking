use dock_core::{NodeId, PanelId, Rect};
use gpui::{App, IntoElement, RenderOnce, Window};
use ui::Theme;

use crate::components::{ClickHandler, IconButton};

/// The group menu button, positioned at the trailing edge of the tab strip.
#[derive(IntoElement)]
pub struct GroupMenuButton {
    /// Group identity used for the stable button element key.
    pub group: NodeId,
    /// Active panel when this menu was requested, if any.
    pub panel: Option<PanelId>,
    /// Menu button geometry calculated for the current frame.
    pub rect: Rect,
    /// Shared application theme.
    pub theme: Theme,
    /// Opens the menu associated with this group and panel.
    pub on_open: ClickHandler,
}

impl RenderOnce for GroupMenuButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let _panel = self.panel;
        IconButton::new(
            format!("dock-group-menu-{:?}", self.group),
            self.rect,
            "☰",
            &self.theme,
            self.on_open,
        )
        .with_border()
    }
}
