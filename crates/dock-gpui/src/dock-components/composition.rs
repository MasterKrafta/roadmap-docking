#[path = "composition_header.rs"]
mod header;
#[path = "group.rs"]
mod group;
#[path = "tab.rs"]
mod tab;
mod dock_column_view;
mod dock_composition_view;

use dock_core::{DockMetrics, GroupRect, Point, Rect};
use gpui::{IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

pub use dock_column_view::DockColumnView;
pub(crate) use dock_composition_view::DockCompositionView;
pub use group::{GroupDragArea, GroupGrip, GroupMenuButton, PanelContent};
pub use tab::DockTab;
pub use header::{
    CloseCompositionButton, CollapseColumnButton, CompositionDragArea, CompositionHeader,
};
pub(crate) use header::CloseCompositionAction;
pub(crate) use group::DockGroupView;

pub(crate) fn group_surface(
    group: GroupRect,
    content_origin: Point,
    metrics: DockMetrics,
    theme: Theme,
) -> impl IntoElement {
    let left = group.rect.origin.x - content_origin.x;
    let top = group.rect.origin.y - content_origin.y;
    let header_height = metrics.header_height.min(group.rect.size.height);
    let content_height = (group.rect.size.height - header_height).max(0.0);

    div()
        .id(format!("dock-group-{:?}", group.group))
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(group.rect.size.width))
        .h(px(group.rect.size.height))
        .border_1()
        .border_color(rgb(theme.colors.group_frame_border))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(header_height))
                .bg(rgb(theme.colors.group_grip_background)),
        )
        .child(
            div()
                .absolute()
                .top(px(header_height))
                .left_0()
                .w_full()
                .h(px(content_height))
                .bg(rgb(theme.colors.panel_surface_background)),
        )
}
