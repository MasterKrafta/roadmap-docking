use dock_core::{Point, Rect};
use gpui::{IntoElement, InteractiveElement, Styled, div, rgb};
use ui::Theme;

/// Flexible drag surface for one complete DockArea composition.
///
/// Its hit region spans the shared header and excludes every Column collapse
/// control plus the single DockArea close control on the right.
pub struct DockAreaDragArea;

impl DockAreaDragArea {
    pub(crate) fn contains(
        area_rect: Rect,
        columns: &[dock_core::ColumnRect],
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
        if point.x >= close_left {
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
