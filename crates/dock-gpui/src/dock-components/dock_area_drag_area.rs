use dock_core::{Point, Rect};
use gpui::{IntoElement, InteractiveElement, Styled, div, rgb};
use ui::Theme;

/// Flexible drag surface for one complete DockArea composition.
///
/// Its hit region spans the shared header and excludes every Column collapse
/// control plus the single DockArea close control on the right.
pub struct DockAreaDragArea;

impl DockAreaDragArea {
    pub(crate) const COLUMN_GRIP_WIDTH: f32 = 24.0;

    pub(crate) fn column_drag_handle_contains(
        column: dock_core::ColumnRect,
        point: Point,
        header_height: f32,
    ) -> bool {
        let width = Self::COLUMN_GRIP_WIDTH.min(column.rect.size.width);
        let height = header_height.min(column.rect.size.height);
        point.x >= column.rect.origin.x
            && point.x < column.rect.origin.x + width
            && point.y >= column.rect.origin.y
            && point.y < column.rect.origin.y + height
    }

    pub(crate) fn contains(
        area_rect: Rect,
        columns: &[dock_core::ColumnRect],
        column_grip_columns: &[dock_core::ColumnRect],
        point: Point,
        header_height: f32,
        button_width: f32,
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

        let close_left = area_right - button_width;
        if point.x >= close_left
            || column_grip_columns.iter().any(|column| {
                Self::column_drag_handle_contains(*column, point, header_height)
            })
        {
            return false;
        }

        columns.iter().all(|column| {
            let collapse_left = Self::collapse_button_left(area_rect, *column, button_width);
            point.x < collapse_left || point.x >= collapse_left + button_width
        })
    }

    pub(crate) fn collapse_button_left(
        area_rect: Rect,
        column: dock_core::ColumnRect,
        button_width: f32,
    ) -> f32 {
        let column_right = column.rect.origin.x + column.rect.size.width;
        let area_right = area_rect.origin.x + area_rect.size.width;
        let is_area_right_edge = (column_right - area_right).abs() < 0.5;
        column_right - area_rect.origin.x
            - if is_area_right_edge {
                2.0 * button_width
            } else {
                button_width
            }
    }

    pub(crate) fn render_column_drag_strip(
        id: String,
        left: f32,
        _theme: Theme,
    ) -> impl IntoElement {
        div()
            .id(id)
            .absolute()
            .left(gpui::px(left))
            .top_0()
            .w(gpui::px(Self::COLUMN_GRIP_WIDTH))
            .h_full()
            .cursor_move()
    }

    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .min_w_0()
            .h_full()
            .cursor_move()
            .bg(rgb(theme.colors.group_grip_background))
    }
}
