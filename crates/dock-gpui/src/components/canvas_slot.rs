use dock_core::Rect;
use gpui::{
    AnyView, IntoElement, ParentElement, Styled, div, prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

/// Positions the application's view in the remaining Core-computed canvas.
///
/// The adapter deliberately does not paint a canvas background or provide
/// document behavior; those belong to the application view. The only adapter
/// paint inside this slot is Core's normalized FreeArea drag preview.
pub struct CanvasSlot;

impl CanvasSlot {
    pub(crate) fn render(
        rect: Rect,
        content: Option<AnyView>,
        free_area_bounds: Option<Rect>,
        theme: Theme,
    ) -> impl IntoElement {
        let mut slot = div()
            .id("dock-canvas-slot")
            .absolute()
            .left(px(rect.origin.x))
            .top(px(rect.origin.y))
            .w(px(rect.size.width))
            .h(px(rect.size.height))
            .overflow_hidden();

        if let Some(content) = content {
            slot = slot.child(div().size_full().child(content));
        }

        if let Some(bounds) = free_area_bounds {
            slot = slot.child(
                div()
                    .absolute()
                    .left(px(bounds.origin.x - rect.origin.x))
                    .top(px(bounds.origin.y - rect.origin.y))
                    .w(px(bounds.size.width))
                    .h(px(bounds.size.height))
                    .bg(rgb(theme.colors.drop_zone))
                    .opacity(0.10)
                    .border_1()
                    .border_color(rgb(theme.colors.drop_zone_border)),
            );
        }

        slot
    }
}
