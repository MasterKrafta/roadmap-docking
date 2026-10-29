//! Visual composition of a dock column's header and content.

use dock_core::{Point, Rect, Size};
use gpui::{Div, ParentElement, Styled, div, px};
use ui::Theme;

use crate::components::MouseDownHandler;

use super::column_header::ColumnHeader;

/// Combines a column header with its recursively rendered child content.
pub(in crate::next_dock) struct DockColumnView {
    pub(in crate::next_dock) width: f32,
    pub(in crate::next_dock) height: f32,
    pub(in crate::next_dock) header_height: f32,
    pub(in crate::next_dock) theme: Theme,
    pub(in crate::next_dock) key: String,
    pub(in crate::next_dock) on_drag_start: MouseDownHandler,
    pub(in crate::next_dock) content: Div,
}

impl DockColumnView {
    pub(in crate::next_dock) fn render(self) -> Div {
        let header_rect = Rect::new(Point::ZERO, Size::new(self.width, self.header_height));

        div()
            .relative()
            .w(px(self.width))
            .h(px(self.height))
            .child(ColumnHeader {
                rect: header_rect,
                theme: self.theme,
                key: self.key,
                on_drag_start: self.on_drag_start,
            })
            .child(self.content)
    }
}
