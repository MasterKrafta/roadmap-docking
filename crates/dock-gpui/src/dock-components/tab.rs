use dock_core::{Point, Rect};
use gpui::{IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Reusable tab header. The active state changes only its visual presentation;
/// pointer behavior is intentionally not connected yet.
pub struct DockTab;

impl DockTab {
    /// Returns true once the pointer leaves the complete TabStrip region.
    ///
    /// The strip includes both tab headers and the empty insertion/drag lane.
    /// Dock adapters use this shared boundary to decide when a tab becomes a
    /// native floating window instead of remaining an in-strip tab drag.
    pub(crate) fn leaves_strip_zone(strip_rect: Rect, point: Point) -> bool {
        !matches!(strip_rect.contains(point), Ok(true))
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
    use dock_core::{Point, Rect, Size};

    use super::DockTab;

    #[test]
    fn tab_detaches_only_after_pointer_leaves_the_complete_strip() {
        let strip = Rect::new(Point::new(10.0, 20.0), Size::new(240.0, 28.0));

        assert!(!DockTab::leaves_strip_zone(strip, Point::new(20.0, 30.0)));
        // The blank lane between tabs remains part of the strip drag zone.
        assert!(!DockTab::leaves_strip_zone(strip, Point::new(180.0, 30.0)));
        assert!(DockTab::leaves_strip_zone(strip, Point::new(9.0, 30.0)));
        assert!(DockTab::leaves_strip_zone(strip, Point::new(20.0, 48.0)));
    }
}
