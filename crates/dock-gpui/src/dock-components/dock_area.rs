#[path = "dock_area_header.rs"]
mod header;

use dock_core::{ColumnRect, DockMetrics, Rect};
use gpui::{InteractiveElement, IntoElement, ParentElement, Styled, div};
use ui::Theme;

pub use header::{
    CollapseColumnButton, CloseDockAreaButton, DockAreaDragArea, DockAreaHeader,
};

/// Root presentation for one Core DockArea.
///
/// A DockArea owns the shared header and the visual composition of its Columns.
/// Core remains the owner of the tree and all geometry.
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

    /// Renders the one shared header above all Columns in this DockArea.
    pub(crate) fn render_header(
        id: String,
        area_rect: Rect,
        columns: &[ColumnRect],
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        DockAreaHeader::render(id, area_rect, columns, metrics, theme)
    }
}
