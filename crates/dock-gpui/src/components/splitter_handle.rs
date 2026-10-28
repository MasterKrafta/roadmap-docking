use dock_core::{Axis, Rect};
use gpui::{
    App, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, RenderOnce,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::with_focus_ring;

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
    /// Handles arrow-key resizing when this is a supported splitter.
    pub on_resize_key: Option<Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App) + 'static>>,
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

        match (self.on_resize_start, self.on_resize_key) {
            (Some(on_resize_start), Some(on_resize_key)) => with_focus_ring(
                element.id(self.id),
                &self.theme,
            )
            .on_key_down(on_resize_key)
            .on_mouse_down(MouseButton::Left, on_resize_start)
            .into_any_element(),
            (Some(on_resize_start), None) => element
                .id(self.id)
                .on_mouse_down(MouseButton::Left, on_resize_start)
                .into_any_element(),
            (None, _) => element.into_any_element(),
        }
    }
}
