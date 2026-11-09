#[path = "dock_area_header.rs"]
mod header;

use dock_core::{ColumnRect, DockMetrics, NodeId, Rect};

use gpui::{InteractiveElement, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

pub use header::{
    CollapseColumnButton, CloseDockAreaButton, DockAreaDragArea, DockAreaHeader,
};

/// Root presentation for one Core DockArea.
///
/// A DockArea owns the shared header and the visual composition of its Columns.
/// Dragging this header transfers the complete Core DockArea presentation through DragHost.
/// This component remains presentation-only; Core owns the tree and all geometry.
pub struct DockAreaView;

impl DockAreaView {
    pub(crate) fn render_root(
    ) -> impl IntoElement + InteractiveElement + ParentElement + Styled {
        div()
            .id("dock-area-root")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    }

    /// Renders the visual placeholder left behind while the complete DockArea is transferred.
    pub(crate) fn render_drag_placeholder(
        id: String,
        area_rect: Rect,
        theme: Theme,
    ) -> impl IntoElement {
        div()
            .id(id)
            .absolute()
            .left(px(area_rect.origin.x))
            .top(px(area_rect.origin.y))
            .w(px(area_rect.size.width))
            .h(px(area_rect.size.height))
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
    }

    /// Renders the one shared header above all Columns in this DockArea.
    pub(crate) fn render_header(
        id: String,
        area_rect: Rect,
        columns: &[ColumnRect],
        dragged_column: Option<NodeId>,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        DockAreaHeader::render(
            id,
            area_rect,
            columns,
            dragged_column,
            metrics,
            theme,
        )
    }
}
