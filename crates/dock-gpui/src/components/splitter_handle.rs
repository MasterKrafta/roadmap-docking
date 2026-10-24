use dock_core::{Axis, Rect};
use gpui::{
    App, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, RenderOnce, Styled, Window,
    div, px, rgb,
};
use ui::Theme;

pub type ResizeStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// The splitter's painted band and its optional resize hit target.
#[derive(IntoElement)]
pub struct SplitterHandle {
    /// Stable identity supplied by the renderer.
    pub id: String,
    /// Absolute bounds, including the hit-target thickness.
    pub rect: Rect,
    /// Axis along which this splitter changes the layout.
    pub axis: Axis,
    /// Shared application theme.
    pub theme: Theme,
    /// Missing when this splitter is visible but not a supported resize boundary.
    pub on_resize_start: Option<ResizeStartHandler>,
}

impl RenderOnce for SplitterHandle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let _axis = self.axis;
        let element = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.splitter));

        if let Some(on_resize_start) = self.on_resize_start {
            element
                .id(self.id)
                .on_mouse_down(MouseButton::Left, on_resize_start)
                .into_any_element()
        } else {
            element.into_any_element()
        }
    }
}
