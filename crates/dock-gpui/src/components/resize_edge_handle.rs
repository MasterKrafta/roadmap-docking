use dock_core::{Rect, ResizeEdge};
use gpui::{
    App, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, RenderOnce, Styled, Window,
    div, px, rgb,
};
use ui::Theme;

use crate::components::{KeyDownHandler, with_focus_ring};

pub type ResizeStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Identifies whether a resize hit zone belongs to a floating edge or an outer dock column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeEdgeKind {
    OuterColumn,
    Floating(ResizeEdge),
    Flyout,
}

/// A resize hit zone that delegates the actual resize gesture to its caller.
#[derive(IntoElement)]
pub struct ResizeEdgeHandle {
    /// Stable prefix for this handle's element ID.
    pub id: String,
    /// Absolute hit-zone geometry supplied by Core or the renderer.
    pub rect: Rect,
    /// Semantic edge kind; the resize request is pre-bound by the caller.
    pub edge: ResizeEdgeKind,
    /// Shared application theme.
    pub theme: Theme,
    /// Begins the resize gesture.
    pub on_resize_start: ResizeStartHandler,
    /// Handles keyboard resizing when this edge is supported for keyboard input.
    pub on_resize_key: Option<KeyDownHandler>,
}

impl RenderOnce for ResizeEdgeHandle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = match self.edge {
            ResizeEdgeKind::OuterColumn | ResizeEdgeKind::Flyout => self.id,
            ResizeEdgeKind::Floating(edge) => format!("{}-{:?}", self.id, edge),
        };

        let element = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.splitter))
            .id(id);

        if let Some(on_resize_key) = self.on_resize_key {
            with_focus_ring(element, &self.theme)
                .on_key_down(on_resize_key)
                .on_mouse_down(MouseButton::Left, self.on_resize_start)
        } else {
            element.on_mouse_down(MouseButton::Left, self.on_resize_start)
        }
    }
}
