use dock_core::{PanelRect, Point};
use gpui::{AnyView, InteractiveElement, IntoElement, ParentElement, Styled, div, px};
use ui::Theme;

use crate::dock_components::PanelContent;

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

            let left = rect.origin.x - coordinate_origin.x;
            let top = rect.origin.y - coordinate_origin.y;
            layer = layer.child(
                div()
                    .id(format!("panel-host-{:?}", panel))
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(rect.size.width))
                    .h(px(rect.size.height))
                    .overflow_hidden()
                    .child(PanelContent::render_with_view(
                        format!("panel-content-{:?}", panel),
                        theme,
                        view,
                    )),
            );

            // TEMP: Draw the PanelRect bounds separately so this outline does
            // not reduce the clipped area or change the panel's layout.
            layer = layer.child(
                div()
                    .id(format!("panel-host-debug-outline-{:?}", panel))
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(rect.size.width))
                    .h(px(rect.size.height))
                    .border_2()
                    .border_color(rgb(0x00FF66)),
            );
        }

        layer
    }
}
