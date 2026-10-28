use dock_core::Rect;
use gpui::{
    App, InteractiveElement, IntoElement, MouseButton, ParentElement, RenderOnce,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{
    ClickHandler, KeyDownHandler, MouseDownHandler,
    focus_ring::{
        is_keyboard_activation_key, keyboard_panel_column_move_direction,
        keyboard_panel_float_move, keyboard_panel_move_direction, keyboard_panel_other_side_move,
        keyboard_panel_rail_move, keyboard_tab_reorder_direction,
    },
    with_focus_ring,
};

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
    pub on_activate: ClickHandler,
    /// Handler invoked when Enter or Space activates the focused tab.
    pub on_activate_key: KeyDownHandler,
    /// Handler invoked when the focused tab is reordered with the keyboard.
    pub on_reorder_key: KeyDownHandler,
    /// Moves this panel to an adjacent group or main-dock column with the keyboard.
    pub on_move_panel_key: Option<KeyDownHandler>,
    /// Handler invoked when the tab is double-clicked.
    pub on_collapse_toggle: ClickHandler,
    /// Handler invoked on left mouse-down to start a panel drag.
    pub on_drag_start: MouseDownHandler,
}

impl RenderOnce for PanelTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let background = if self.active {
            self.theme.colors.active_tab
        } else {
            self.theme.colors.toolbar_background
        };
        let hover_background = if self.active {
            background
        } else {
            self.theme.colors.hover
        };
        let typography = self.theme.typography;

        let tab = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(background))
            .text_color(rgb(self.theme.colors.primary_text))
            .font_family(typography.family)
            .text_size(px(typography.tab_size))
            .line_height(px(typography.line_height))
            .child(self.label);

        let on_activate = self.on_activate;
        let on_collapse_toggle = self.on_collapse_toggle;
        let on_activate_key = self.on_activate_key;
        let on_reorder_key = self.on_reorder_key;
        let on_move_panel_key = self.on_move_panel_key;

        with_focus_ring(tab.id(self.id), &self.theme)
            .hover(move |style| style.bg(rgb(hover_background)))
            .on_key_down(move |event, window, app| {
                if is_keyboard_activation_key(&event.keystroke.key) {
                    on_activate_key(event, window, app);
                } else if keyboard_tab_reorder_direction(&event.keystroke).is_some() {
                    on_reorder_key(event, window, app);
                } else if (keyboard_panel_move_direction(&event.keystroke).is_some()
                    || keyboard_panel_column_move_direction(&event.keystroke).is_some()
                    || keyboard_panel_other_side_move(&event.keystroke)
                    || keyboard_panel_rail_move(&event.keystroke)
                    || keyboard_panel_float_move(&event.keystroke))
                    && let Some(handler) = &on_move_panel_key
                {
                    handler(event, window, app);
                }
            })
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
