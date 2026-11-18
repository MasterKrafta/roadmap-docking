use dock_core::{PanelId, PanelRect, Point};
use gpui::{
    AnyView, Entity, InteractiveElement, IntoElement, ParentElement, Styled, Window, WindowId, div,
    px, rgb,
};
use ui::Theme;

use crate::{DockController, dock_components::PanelContent};

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
    /// Confirms that transferred views are no longer mounted by this source host.
    ///
    /// DragHost may mount a transferred PanelView only after this window has
    /// rendered its placeholder (or its replacement active tab).
    pub(crate) fn confirm_source_detached_after_render(
        controller: &Entity<DockController>,
        window: &mut Window,
        source_window_id: WindowId,
        pending_transfers: Vec<PanelId>,
        panel_views: &[(PanelRect, Option<AnyView>)],
    ) {
        if pending_transfers.is_empty()
            || !pending_transfers.iter().all(|panel| {
                panel_views
                    .iter()
                    .all(|(rect, view)| rect.panel != *panel || view.is_none())
            })
        {
            return;
        }

        let controller = controller.clone();
        window.on_next_frame(move |_, app| {
            controller.update(app, |controller, controller_cx| {
                let mut confirmed = false;
                for panel in pending_transfers {
                    confirmed |=
                        controller.confirm_panel_view_source_detached(panel, source_window_id);
                }
                if confirmed {
                    controller_cx.notify();
                }
            });
        });
    }

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
