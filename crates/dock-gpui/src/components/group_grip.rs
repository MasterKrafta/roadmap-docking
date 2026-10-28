use dock_core::{NodeId, Rect};
use gpui::{
    App, IntoElement, InteractiveElement, KeyDownEvent, MouseButton, MouseDownEvent, RenderOnce,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{focus_ring::keyboard_group_move_direction, with_focus_ring};

/// A narrow, full-width drag target used above a group's tab strip.
#[derive(IntoElement)]
pub struct GroupGrip {
    /// Tabs-group identity for a stable element key.
    pub group: NodeId,
    /// Absolute grip bounds supplied by the renderer.
    pub rect: Rect,
    /// Shared application theme.
    pub theme: Theme,
    /// Begins dragging the group.
    pub on_drag_start: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    /// Moves the group within its column when the keyboard shortcut is pressed.
    pub on_move_key: Option<Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App) + 'static>>,
}

impl RenderOnce for GroupGrip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let element = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.toolbar_background))
            .id(format!("dock-group-grip-{:?}", self.group));

        if let Some(on_move_key) = self.on_move_key {
            with_focus_ring(element, &self.theme)
                .on_key_down(move |event, window, app| {
                    if keyboard_group_move_direction(&event.keystroke).is_some() {
                        on_move_key(event, window, app);
                    }
                })
                .on_mouse_down(MouseButton::Left, self.on_drag_start)
        } else {
            element.on_mouse_down(MouseButton::Left, self.on_drag_start)
        }
    }
}
