//! Visual frame for one floating dock composition.

use dock_core::Rect;
use gpui::{App, Div, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, rgb};
use ui::Theme;

/// Places floating composition content at its current bounds and paints its frame.
#[derive(IntoElement)]
pub(super) struct FloatingCompositionView {
    pub(super) bounds: Rect,
    pub(super) theme: Theme,
    pub(super) content: Div,
}

impl RenderOnce for FloatingCompositionView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bounds = self.bounds;

        div()
            .absolute()
            .left(px(bounds.origin.x))
            .top(px(bounds.origin.y))
            .w(px(bounds.size.width))
            .h(px(bounds.size.height))
            .border_1()
            .border_color(rgb(self.theme.colors.border))
            .child(self.content)
    }
}
