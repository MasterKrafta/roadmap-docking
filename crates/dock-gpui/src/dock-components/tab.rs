use dock_core::{DockMetrics, NodeId, Point, Rect, TabHeaderRect};
use gpui::{
    IntoElement, InteractiveElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, div, img, px, rgb,
};
use ui::Theme;


/// Reusable tab header. The shared DockHost/FloatingHost pointer pipelines
/// handle its drag gesture from Core tab geometry.
pub struct DockTab;

impl DockTab {
    /// Computes the drag zone of the new DockColumn group header.
    ///
    /// It contains the tab, the flexible spacer, and the GroupGrip. Only the
    /// trailing GroupMenuButton is excluded from tab dragging.
    pub(crate) fn drag_zone(group_rect: Rect, metrics: DockMetrics) -> Rect {
        Rect::new(
            group_rect.origin,
            dock_core::Size::new(
                (group_rect.size.width - metrics.group_control_button_width).max(0.0),
                metrics
                    .header_height
                    .max(metrics.tab_strip_height)
                    .min(group_rect.size.height),
            ),
        )
    }

    /// Returns true once the pointer leaves the complete DockTab drag zone.
    pub(crate) fn leaves_strip_zone(strip_rect: Rect, point: Point) -> bool {
        !matches!(strip_rect.contains(point), Ok(true))
    }

    /// Renders the visual tab surface from Core geometry. Activation and drag are
    /// handled by the shared host pointer pipeline; closing lives in group controls.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_core(
        group: NodeId,
        header: TabHeaderRect,
        strip_rect: Rect,
        title: SharedString,
        icon: Option<SharedString>,
        active: bool,
        theme: Theme,
    ) -> impl IntoElement {
        let left = header.rect.origin.x - strip_rect.origin.x;
        let top = header.rect.origin.y - strip_rect.origin.y;
        let width = header.rect.size.width.max(0.0);
        let height = header.rect.size.height.max(0.0);

        let mut title_row = div()
            .absolute()
            .left(px(6.0))
            .right_0()
            .top_0()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size));

        if let Some(icon) = icon {
            title_row = title_row.child(img(icon).size(px(14.0)).flex_none());
        }

        title_row = title_row.child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(title),
        );

        let tab = div()
            .id(format!("tab-{:?}-{:?}", group, header.panel))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .bg(rgb(if active {
                theme.colors.panel_tab_active
            } else {
                theme.colors.panel_tab_inactive
            }))
            .text_color(rgb(theme.colors.primary_text))
            .cursor_pointer()
            .child(title_row);

        tab
    }

    pub fn render(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
    ) -> impl IntoElement {
        Self::render_with_metrics(id, title, active, theme, 112.0, 72.0, 8.0)
    }

    pub fn render_with_width(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
        width: f32,
    ) -> impl IntoElement {
        Self::render_with_metrics(id, title, active, theme, width, width.min(72.0), 8.0)
    }

    pub fn render_with_metrics(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
        width: f32,
        min_width: f32,
        horizontal_padding: f32,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w(px(width))
            .min_w(px(min_width))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .cursor_move()
            .px(px(horizontal_padding))
            .overflow_hidden()
            .bg(rgb(if active {
                theme.colors.panel_tab_active
            } else {
                theme.colors.panel_tab_inactive
            }))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            )
    }
}

#[cfg(test)]
mod tests {
    use dock_core::{DockMetrics, Point, Rect, Size};

    use super::DockTab;

    #[test]
    fn tab_drag_zone_includes_the_group_grip_but_excludes_the_group_menu() {
        let group = Rect::new(Point::new(10.0, 20.0), Size::new(240.0, 160.0));
        let zone = DockTab::drag_zone(group, DockMetrics::default());

        assert_eq!(zone.origin, Point::new(10.0, 20.0));
        assert_eq!(zone.size, Size::new(212.0, 28.0));
        assert!(!DockTab::leaves_strip_zone(zone, Point::new(20.0, 30.0)));
        // The blank lane remains part of the drag zone.
        assert!(!DockTab::leaves_strip_zone(zone, Point::new(180.0, 30.0)));
        // The GroupGrip (32 px) is included; the 28 px GroupMenuButton is not.
        assert!(!DockTab::leaves_strip_zone(zone, Point::new(215.0, 30.0)));
        assert!(DockTab::leaves_strip_zone(zone, Point::new(222.0, 30.0)));
        assert!(DockTab::leaves_strip_zone(zone, Point::new(20.0, 48.0)));
    }
}
