use dock_core::{Axis, Point, Rect, SplitterRect};
use gpui::{IntoElement, InteractiveElement, Styled, div, rgb};
use ui::Theme;

/// Flexible drag surface for a Column header or a complete legacy composition header.
///
/// The hit region includes only actual Column header rectangles and excludes
/// the controls and horizontal splitters supplied by the host.
pub struct CompositionDragArea;

impl CompositionDragArea {
    pub(crate) fn contains(
        area_rect: Rect,
        columns: &[dock_core::ColumnRect],
        splitters: &[SplitterRect],
        point: Point,
        header_height: f32,
        button_width: f32,
        show_close_button: bool,
    ) -> bool {
        let header_height = header_height.min(area_rect.size.height);
        let area_right = area_rect.origin.x + area_rect.size.width;
        if point.y < area_rect.origin.y
            || point.y >= area_rect.origin.y + header_height
            || point.x < area_rect.origin.x
            || point.x >= area_right
        {
            return false;
        }

        let inside_column_header = columns.iter().any(|column| {
            let column_header_height = header_height.min(column.rect.size.height);
            point.x >= column.rect.origin.x
                && point.x < column.rect.origin.x + column.rect.size.width
                && point.y >= column.rect.origin.y
                && point.y < column.rect.origin.y + column_header_height
        });
        if !inside_column_header {
            return false;
        }

        if show_close_button && point.x >= area_right - button_width {
            return false;
        }

        // X splitters remain independent resize handles where they cross a header.
        if splitters.iter().any(|splitter| {
            splitter.axis == Axis::X && matches!(splitter.rect.contains(point), Ok(true))
        }) {
            return false;
        }

        columns.iter().all(|column| {
            let collapse_left =
                Self::collapse_button_left(area_rect, *column, button_width, show_close_button);
            point.x < collapse_left || point.x >= collapse_left + button_width
        })
    }

    pub(crate) fn collapse_button_left(
        area_rect: Rect,
        column: dock_core::ColumnRect,
        button_width: f32,
        show_close_button: bool,
    ) -> f32 {
        let column_right = column.rect.origin.x + column.rect.size.width;
        let area_right = area_rect.origin.x + area_rect.size.width;
        let is_area_right_edge = (column_right - area_right).abs() < 0.5;
        let reserved_width = if show_close_button && is_area_right_edge {
            2.0 * button_width
        } else {
            button_width
        };
        column_right - area_rect.origin.x - reserved_width
    }

    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .absolute()
            .left_0()
            .top_0()
            .w_full()
            .h_full()
            .cursor_move()
            .bg(rgb(theme.colors.group_grip_background))
    }
}
