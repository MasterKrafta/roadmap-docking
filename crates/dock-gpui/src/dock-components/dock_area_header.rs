#[path = "dock_area_drag_area.rs"]
mod drag_area;
#[path = "collapse_column_button.rs"]
mod collapse_column_button;
#[path = "close_dock_area_button.rs"]
mod close_dock_area_button;

use dock_core::{ColumnRect, DockMetrics, NodeId, Rect};

use gpui::{InteractiveElement, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

pub use collapse_column_button::CollapseColumnButton;
pub use close_dock_area_button::CloseDockAreaButton;
pub use drag_area::DockAreaDragArea;

/// Shared header for one complete DockArea composition.
///
/// Each Column has one collapse control aligned to its right edge, while the
/// whole composition has a single close control at the far right.
pub struct DockAreaHeader;

impl DockAreaHeader {
    pub(crate) fn render(
        id: String,
        area_rect: Rect,
        columns: &[ColumnRect],
        dragged_column: Option<NodeId>,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let button_width = metrics.column_control_button_width;
        let mut header = div()
            .id(format!("{id}-header"))
            .absolute()
            .left(px(area_rect.origin.x))
            .top(px(area_rect.origin.y))
            .w(px(area_rect.size.width))
            .h(px(metrics.column_header_height))
            .flex()
            .flex_none()
            .items_stretch()
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(DockAreaDragArea::render(
                format!("{id}-drag-area"),
                theme,
            ));

        for (index, column) in columns.iter().enumerate() {
            if dragged_column != Some(column.column) {
                header = header.child(DockAreaDragArea::render_column_drag_strip(
                    format!("{id}-column-grip-{index}"),
                    column.rect.origin.x - area_rect.origin.x,
                    theme,
                ));
            }

            if dragged_column == Some(column.column) {
                continue;
            }
            let button_x =
                DockAreaDragArea::collapse_button_left(area_rect, *column, button_width);

            header = header.child(
                div()
                    .absolute()
                    .left(px(button_x))
                    .top_0()
                    .h_full()
                    .child(CollapseColumnButton::render_with_metrics(
                        format!("{id}-collapse-column-{index}"),
                        theme,
                        button_width,
                        metrics.control_icon_size,
                    )),
            );
        }

        header.child(
            div()
                .absolute()
                .left(px((area_rect.size.width - button_width).max(0.0)))
                .top_0()
                .h_full()
                .child(CloseDockAreaButton::render_with_metrics(
                    format!("{id}-close"),
                    theme,
                    button_width,
                    metrics.control_icon_size,
                )),
        )
    }
}
