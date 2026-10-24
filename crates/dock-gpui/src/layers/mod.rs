pub(crate) mod drop_overlay;
pub(crate) mod floating;
pub(crate) mod flyout;
pub(crate) mod main_areas;
pub(crate) mod outer_edges;
pub(crate) mod tree;

use dock_core::Rect;
use gpui::{Div, Styled, div, px, rgb};

/// Shared primitive for an absolute color rectangle used by frame layers.
pub(crate) fn rect_element(rect: Rect, color: u32) -> Div {
    div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height))
        .bg(rgb(color))
}
