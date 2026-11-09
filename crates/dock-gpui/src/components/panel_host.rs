use dock_core::{PanelRect, Point};
use gpui::{
    AnyView, IntoElement, ParentElement, Styled, div, prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

/// Places cached panel views in the rectangles emitted by dock-core.
///
/// The input contains only views selected for mounting by DockHost. Core emits
/// geometry only for the active child of each Tabs group, so inactive cached
/// views are not mounted and cannot receive pointer input.
///
/// coordinate_origin is the origin of the containing coordinate space. It is
/// the content origin for a scrolling ColumnView and Point::ZERO for floating
/// panels positioned in the workspace.
pub struct PanelHost;

impl PanelHost {
    pub(crate) fn render(
        panels: Vec<(PanelRect, Option<AnyView>)>,
        coordinate_origin: Point,
        theme: Theme,
    ) -> impl IntoElement {
        let mut layer = div().absolute().top_0().left_0().size_full();

        for (panel_rect, view) in panels {
            let panel = panel_rect.panel;
            let rect = panel_rect.rect;

            let content = match view {
                Some(view) => div().size_full().child(view),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(theme.colors.panel_surface_background))
                    .border_1()
                    .border_color(rgb(theme.colors.group_frame_border))
                    .text_color(rgb(theme.colors.primary_text))
                    .child("Перемещение панели…"),
            };

            layer = layer.child(
                div()
                    .id(format!("panel-host-{:?}", panel))
                    .absolute()
                    .left(px(rect.origin.x - coordinate_origin.x))
                    .top(px(rect.origin.y - coordinate_origin.y))
                    .w(px(rect.size.width))
                    .h(px(rect.size.height))
                    .overflow_hidden()
                    .child(content),
            );
        }

        layer
    }
}
