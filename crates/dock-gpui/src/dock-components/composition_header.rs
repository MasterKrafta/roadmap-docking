#[path = "composition_drag_area.rs"]
mod drag_area;
#[path = "collapse_column_button.rs"]
mod collapse_column_button;
#[path = "close_composition_button.rs"]
mod close_composition_button;

use dock_core::{ColumnRect, DockMetrics, NodeId, Rect};

use gpui::{InteractiveElement, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

pub use collapse_column_button::CollapseColumnButton;
pub use close_composition_button::CloseCompositionButton;
pub use drag_area::CompositionDragArea;

/// Shared header for one complete composition.
///
/// Each Column has one collapse control aligned to its right edge, while the
/// whole composition has a single close control at the far right.
pub struct CompositionHeader;

impl CompositionHeader {
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
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(CompositionDragArea::render(
                format!("{id}-drag-area"),
                theme,
            ));

        for (index, column) in columns.iter().enumerate() {
            if dragged_column != Some(column.column) {
                header = header.child(CompositionDragArea::render_column_drag_strip(
                    format!("{id}-column-drag-strip-{index}"),
                    column.rect.origin.x - area_rect.origin.x,
                ));
            }

            if dragged_column == Some(column.column) {
                continue;
            }
            let button_x =
                CompositionDragArea::collapse_button_left(area_rect, *column, button_width);

            header = header.child(
                div()
                    .absolute()
                    .left(px(button_x))
                    .top_0()
                    .w(px(button_width))
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
                .w(px(button_width))
                .h_full()
                .child(CloseCompositionButton::render_with_metrics(
                    format!("{id}-close"),
                    theme,
                    button_width,
                    metrics.control_icon_size,
                )),
        )
    }
}
