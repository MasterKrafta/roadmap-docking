use dock_core::{NodeId, PanelId, Point, Rect, Size};
use gpui::{
    App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{ClickHandler, IconButton};

/// Header bar for a flyout, including its menu and close controls.
#[derive(IntoElement)]
pub struct FlyoutHeader {
    /// Group identity used to keep the menu request tied to its dock group.
    pub group: NodeId,
    /// Panel shown by the flyout.
    pub panel: PanelId,
    /// Header bounds in flyout-local coordinates.
    pub rect: Rect,
    /// Title shown at the leading edge of the header.
    pub title: String,
    /// Shared application theme.
    pub theme: Theme,
    /// Requests the group/panel menu.
    pub on_menu: ClickHandler,
    /// Closes the currently displayed flyout.
    pub on_close: ClickHandler,
}

impl RenderOnce for FlyoutHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let button_width = (self.rect.size.width / 2.0).min(28.0);
        let mut header = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.toolbar_background))
            .text_color(rgb(self.theme.colors.primary_text))
            .child(self.title);

        let menu_rect = Rect::new(
            Point::new(self.rect.size.width - button_width * 2.0, 0.0),
            Size::new(button_width, self.rect.size.height),
        );
        header = header.child(
            IconButton::new(
                format!("dock-flyout-menu-{:?}-{:?}", self.group, self.panel),
                menu_rect,
                "⋯",
                &self.theme,
                self.on_menu,
            )
            .into_any_element(),
        );

        let close_rect = Rect::new(
            Point::new(self.rect.size.width - button_width, 0.0),
            Size::new(button_width, self.rect.size.height),
        );
        header.child(
            IconButton::new(
                format!("dock-flyout-close-{:?}-{:?}", self.group, self.panel),
                close_rect,
                "×",
                &self.theme,
                self.on_close,
            )
            .into_any_element(),
        )
    }
}
