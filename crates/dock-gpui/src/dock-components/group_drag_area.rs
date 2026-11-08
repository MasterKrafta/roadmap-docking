use dock_core::{DockMetrics, Point, Rect, Size};
use gpui::{IntoElement, InteractiveElement, Styled, div, rgb};
use ui::Theme;

/// Flexible empty drag surface for the whole Group.
pub struct GroupDragArea;

impl GroupDragArea {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .min_w_0()
            .h_full()
            .cursor_move()
            .bg(rgb(theme.colors.group_grip_background))
    }

    /// Returns the complete draggable Group header, including GroupGrip and
    /// the flexible drag lane, while leaving the trailing controls outside.
    pub(crate) fn drag_zone(group: Rect, metrics: DockMetrics, in_column: bool) -> Rect {
        let control_width = metrics
            .group_control_button_width
            .min(group.size.width / 3.0)
            .max(1.0);
        let control_count = if in_column { 1.0 } else { 2.0 };
        let reserved_right = (control_width * control_count).min(group.size.width);
        let header_height = metrics
            .header_height
            .max(metrics.tab_strip_height)
            .min(group.size.height);

        Rect::new(
            group.origin,
            Size::new(
                (group.size.width - reserved_right).max(0.0),
                header_height.max(0.0),
            ),
        )
    }

    pub(crate) fn contains(zone: Rect, point: Point) -> bool {
        matches!(zone.contains(point), Ok(true))
    }
}

#[cfg(test)]
mod tests {
    use dock_core::{DockMetrics, Point, Rect, Size};

    use super::GroupDragArea;

    #[test]
    fn column_group_drag_zone_includes_group_grip_but_excludes_menu() {
        let metrics = DockMetrics::default();
        let group = Rect::new(Point::new(20.0, 30.0), Size::new(300.0, 180.0));
        let zone = GroupDragArea::drag_zone(group, metrics, true);

        let control_width = metrics.group_control_button_width.min(100.0).max(1.0);
        assert_eq!(zone.origin, group.origin);
        assert_eq!(zone.size.width, group.size.width - control_width);
        assert_eq!(
            zone.size.height,
            metrics.header_height.max(metrics.tab_strip_height)
        );
        assert!(GroupDragArea::contains(
            zone,
            Point::new(group.origin.x + group.size.width - control_width - 1.0, group.origin.y + 1.0)
        ));
        assert!(!GroupDragArea::contains(
            zone,
            Point::new(group.origin.x + group.size.width - control_width, group.origin.y + 1.0)
        ));
    }

    #[test]
    fn standalone_group_drag_zone_leaves_both_trailing_controls_outside() {
        let metrics = DockMetrics::default();
        let group = Rect::new(Point::new(20.0, 30.0), Size::new(300.0, 180.0));
        let zone = GroupDragArea::drag_zone(group, metrics, false);
        let control_width = metrics.group_control_button_width.min(100.0).max(1.0);

        assert_eq!(zone.size.width, group.size.width - 2.0 * control_width);
    }
}
