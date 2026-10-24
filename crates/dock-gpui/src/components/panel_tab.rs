use dock_core::Rect;
use gpui::{
    App, ClickEvent, IntoElement, MouseButton, MouseDownEvent, ParentElement, RenderOnce, Styled,
    Window, div, px, rgb,
};
use ui::Theme;

use crate::components::with_focus_ring;

/// A single tab header that delegates all dock actions to caller-provided handlers.
#[derive(IntoElement)]
pub struct PanelTab {
    /// Stable element identity supplied by the renderer.
    pub id: String,
    /// Absolute position and size supplied by Core's frame geometry.
    pub rect: Rect,
    /// Display label, already normalized for collapsed groups by the renderer.
    pub label: String,
    /// Whether this tab is the active panel in its group.
    pub active: bool,
    /// A copy of the shared theme palette and metrics.
    pub theme: Theme,
    /// Handler invoked when the tab is clicked once.
    pub on_activate: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    /// Handler invoked when the tab is double-clicked.
    pub on_collapse_toggle: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    /// Handler invoked on left mouse-down to start a panel drag.
    pub on_drag_start: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
}

impl RenderOnce for PanelTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let background = if self.active {
            self.theme.colors.active_tab
        } else {
            self.theme.colors.toolbar_background
        };

        let tab = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(background))
            .text_color(rgb(self.theme.colors.primary_text))
            .child(self.label);

        let on_activate = self.on_activate;
        let on_collapse_toggle = self.on_collapse_toggle;

        with_focus_ring(tab.id(self.id), &self.theme)
            .on_mouse_down(MouseButton::Left, self.on_drag_start)
            .on_click(move |event, window, app| {
                if event.click_count() >= 2 {
                    on_collapse_toggle(event, window, app);
                } else {
                    on_activate(event, window, app);
                }
            })
    }
}
