use dock_core::{Axis, Point, SplitterRect};
use gpui::{IntoElement, div, px, rgb};
use ui::Theme;

pub(crate) fn splitter_surface(
    splitter: SplitterRect,
    content_origin: Point,
    theme: Theme,
) -> impl IntoElement {
    let hit_rect = splitter.rect;
    let thickness = theme.metrics.splitter_thickness;
    let (left, top, width, height) = match splitter.axis {
        Axis::X => (
            hit_rect.origin.x + (hit_rect.size.width - thickness) * 0.5 - content_origin.x,
            hit_rect.origin.y - content_origin.y,
            thickness,
            hit_rect.size.height,
        ),
        Axis::Y => (
            hit_rect.origin.x - content_origin.x,
            hit_rect.origin.y + (hit_rect.size.height - thickness) * 0.5 - content_origin.y,
            hit_rect.size.width,
            thickness,
        ),
    };

    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width.max(0.0)))
        .h(px(height.max(0.0)))
        .bg(rgb(theme.colors.splitter_handle))
}

