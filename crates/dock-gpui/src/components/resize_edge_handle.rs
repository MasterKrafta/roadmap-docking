use dock_core::{Rect, ResizeEdge};
use gpui::{
    App, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, RenderOnce, Styled, Window,
    div, px, rgb,
};
use ui::Theme;

pub type ResizeStartHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// Identifies whether a resize hit zone belongs to a floating edge or an outer dock column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeEdgeKind {
    OuterColumn,
    Floating(ResizeEdge),
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
}

impl RenderOnce for ResizeEdgeHandle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = match self.edge {
            ResizeEdgeKind::OuterColumn => self.id,
            ResizeEdgeKind::Floating(edge) => format!("{}-{:?}", self.id, edge),
        };

        div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.theme.colors.splitter))
            .id(id)
            .on_mouse_down(MouseButton::Left, self.on_resize_start)
    }
}
