use dock_core::{NodeId, Rect};
use gpui::{
    App, IntoElement, InteractiveElement, MouseButton, MouseDownEvent, RenderOnce, Styled, Window,
    div, px, rgb,
};
use ui::Theme;

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
}

impl RenderOnce for GroupGrip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.toolbar_background))
            .id(format!("dock-group-grip-{:?}", self.group))
            .on_mouse_down(MouseButton::Left, self.on_drag_start)
    }
}
