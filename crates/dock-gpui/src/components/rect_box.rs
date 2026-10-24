use dock_core::Rect;
use gpui::{AnyElement, App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, rgb};

/// An absolutely positioned rectangle with an optional child element.
///
/// This is the reusable base primitive for geometry-only dock surfaces. More
/// specialized controls can compose it or use the same geometry directly.
#[derive(IntoElement)]
pub struct RectBox {
    rect: Rect,
    color: u32,
    child: Option<AnyElement>,
}

impl RectBox {
    /// Creates a rectangle at the supplied logical-pixel bounds.
    pub fn new(rect: Rect, color: u32) -> Self {
        Self {
            rect,
            color,
            child: None,
        }
    }

    /// Places a child within the rectangle without changing its bounds.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }
}

impl RenderOnce for RectBox {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let element = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(self.color));

        if let Some(child) = self.child {
            element.child(child)
        } else {
            element
        }
    }
}
