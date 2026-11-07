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
        point: Point,
        header_height: f32,
        reserved_controls_width: f32,
    ) -> bool {
        let header_height = header_height.min(area_rect.size.height);
        let controls_width = reserved_controls_width.min(area_rect.size.width.max(0.0));

        point.y >= area_rect.origin.y
            && point.y < area_rect.origin.y + header_height
            && point.x >= area_rect.origin.x
            && point.x < area_rect.origin.x + area_rect.size.width - controls_width
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
