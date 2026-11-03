use dock_core::{Point, Rect};
use gpui::{IntoElement, InteractiveElement, Styled, div, rgb};
use ui::Theme;

/// Flexible drag surface for the whole Column header.
///
/// The same geometry is used by DockHost's shared pointer pipeline, so the
/// visible surface and the semantic ColumnHeader hit zone cannot drift apart.
pub struct ColumnDragArea;

impl ColumnDragArea {
    /// Returns whether a point belongs to the flexible header area, excluding
    /// the fixed controls on its right.
    pub(crate) fn contains(
        column_rect: Rect,
        point: Point,
        header_height: f32,
        reserved_controls_width: f32,
    ) -> bool {
        let header_height = header_height.min(column_rect.size.height);
        let controls_width = reserved_controls_width
            .min(column_rect.size.width / 3.0)
            .max(1.0);

        point.y >= column_rect.origin.y
            && point.y < column_rect.origin.y + header_height
            && point.x >= column_rect.origin.x
            && point.x < column_rect.origin.x + column_rect.size.width - controls_width
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
