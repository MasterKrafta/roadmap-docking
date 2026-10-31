use dock_core::{Axis, DockMetrics, FloatingGroupId, Point, SplitterRect};
use gpui::{
    Entity, IntoElement, ParentElement, Styled, div, prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::DockController;

/// Splitter visuals are driven by Core geometry. DockHost owns all pointer events.
pub struct SplitterView;

impl SplitterView {
    pub(crate) fn render(
        splitter: SplitterRect,
        content_origin: Point,
        _pointer_offset: Point,
        _controller: Entity<DockController>,
        metrics: DockMetrics,
        theme: Theme,
        _raise_floating: Option<FloatingGroupId>,
    ) -> impl IntoElement {
        let hit_rect = splitter.rect;
        let thickness = metrics.splitter_thickness;
        let (left, top, width, height, line_left, line_top, line_width, line_height) =
            match splitter.axis {
                Axis::X => (
                    hit_rect.origin.x - content_origin.x,
                    hit_rect.origin.y - content_origin.y,
                    hit_rect.size.width,
                    hit_rect.size.height,
                    (hit_rect.size.width - thickness) * 0.5,
                    0.0,
                    thickness,
                    hit_rect.size.height,
                ),
                Axis::Y => (
                    hit_rect.origin.x - content_origin.x,
                    hit_rect.origin.y - content_origin.y,
                    hit_rect.size.width,
                    hit_rect.size.height,
                    0.0,
                    (hit_rect.size.height - thickness) * 0.5,
                    hit_rect.size.width,
                    thickness,
                ),
            };

        let mut hitbox = div()
            .id(format!(
                "splitter-{:?}-{}-{:?}",
                splitter.owner, splitter.boundary_index, splitter.axis
            ))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height));

        hitbox = match splitter.axis {
            Axis::X => hitbox.cursor_col_resize(),
            Axis::Y => hitbox.cursor_row_resize(),
        };
        hitbox.child(
            div()
                .absolute()
                .left(px(line_left))
                .top(px(line_top))
                .w(px(line_width.max(0.0)))
                .h(px(line_height.max(0.0)))
                .bg(rgb(theme.colors.splitter_handle)),
        )
    }
}
