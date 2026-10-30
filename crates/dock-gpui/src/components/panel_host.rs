use dock_core::{PanelRect, Point};
use gpui::{App, Entity, IntoElement, ParentElement, Styled, Window, div, px};

use crate::DockController;

/// Mounts Core-emitted active panel content in its allocated rectangle.
///
/// Core emits a PanelRect only for the active child of each Tabs group. Cached
/// inactive views therefore stay alive in DockController but are not mounted
/// into the element tree and cannot receive pointer input.
///
/// The coordinate origin lets the same component work inside a scrolling
/// ColumnView as well as in the workspace layer for floating compositions.
/// PanelState remains Core-owned; this component does not write measured sizes
/// back into the model.
pub struct PanelHost;

impl PanelHost {
    pub(crate) fn render(
        panels: &[PanelRect],
        coordinate_origin: Point,
        controller: Entity<DockController>,
        window: &mut Window,
        app: &mut App,
    ) -> impl IntoElement {
        let mut layer = div().absolute().top_0().left_0().size_full();

        for panel_rect in panels {
            let panel = panel_rect.panel;
            let rect = panel_rect.rect;
            let view = controller.update(app, |controller, panel_cx| {
                controller.panel_view(panel, window, panel_cx)
            });
            let Some(view) = view else {
                continue;
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
                    .child(div().size_full().child(view)),
            );
        }

        layer
    }
}
