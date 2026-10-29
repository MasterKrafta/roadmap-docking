//! Composition of the frame, content surface, and header for one tab group.

use dock_core::{Point, Rect, Size};
use gpui::{Div, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

use crate::components::{GroupFrame, PanelSurface};

use super::super::model::TabGroup;
use super::super::window::{TabDragContext, TabReorderPreview};
use super::group_header::GroupHeader;

/// Builds the visual tree for one tab group without changing its behavior.
pub(in crate::next_dock) fn render(
    group: &TabGroup,
    width: f32,
    height: f32,
    tab_height: f32,
    content_height: f32,
    theme: &Theme,
    key: &str,
    tab_drag_context: Option<TabDragContext>,
    tab_reorder_preview: Option<TabReorderPreview>,
) -> Div {
    let group_rect = Rect::new(Point::ZERO, Size::new(width, height));
    let strip_rect = Rect::new(Point::ZERO, Size::new(width, tab_height));
    let content_rect = Rect::new(
        Point::new(0.0, tab_height),
        Size::new(width, content_height),
    );

    let content = match group.panels.get(group.active) {
        Some(active) => div()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_color(rgb(theme.colors.primary_text))
                    .child(active.title),
            )
            .child(
                div()
                    .text_color(rgb(theme.colors.muted_text))
                    .child(active.description),
            )
            .into_any_element(),
        None => div().child("Empty panel group").into_any_element(),
    };

    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .child(GroupFrame {
            rect: group_rect,
            theme: *theme,
        })
        .child(PanelSurface {
            rect: content_rect,
            theme: *theme,
            content,
        })
        .child(GroupHeader {
            rect: strip_rect,
            group: group.clone(),
            theme: *theme,
            key: key.to_owned(),
            tab_drag_context,
            tab_reorder_preview,
        })
}
