//! Visual composition of a dock column's header and content.

use dock_core::{Point, Rect, Size};
use gpui::{App, Div, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use ui::Theme;

use crate::components::MouseDownHandler;

use super::column_header::ColumnHeader;

/// Combines a column header with its recursively rendered child content.
#[derive(IntoElement)]
pub(in crate::next_dock) struct DockColumnView {
    pub(in crate::next_dock) width: f32,
    pub(in crate::next_dock) height: f32,
    pub(in crate::next_dock) header_height: f32,
    pub(in crate::next_dock) theme: Theme,
    pub(in crate::next_dock) key: String,
    pub(in crate::next_dock) on_drag_start: MouseDownHandler,
    pub(in crate::next_dock) content: Div,
}

impl RenderOnce for DockColumnView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let header_rect = Rect::new(
            Point::ZERO,
            Size::new(self.width, self.header_height),
        );

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
