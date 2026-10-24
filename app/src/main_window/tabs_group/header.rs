use super::{close_button, collapse_button};
use super::super::{DockDragPreview, GroupDragPayload};
use dock_core::NodeId;
use gpui::{
    AppContext, CursorStyle, Div, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, div, px,
    rgb,
};

/// Header controls stay outside the draggable area; only the free area carries the dock-tree drag.
pub(super) fn render(
    theme: ui::Theme,
    width: f32,
    height: f32,
    group_id: NodeId,
    dock_tree_drag: Option<GroupDragPayload>,
) -> Div {
    let drag_area = div()
        .flex_1()
        .h(px(height))
        .id(format!("dock-tree-header-drag-zone-{group_id:?}"));

    let drag_area = match dock_tree_drag {
        Some(payload) => drag_area.cursor(CursorStyle::OpenHand).on_drag(payload, |payload, _cursor_offset, window, cx| {
            payload.drag_start_position.set(window.mouse_position());
            cx.new(|_| DockDragPreview)
        }),
        None => drag_area,
    };

    div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(drag_area)
        .child(collapse_button::render(theme, height))
        .child(close_button::render(theme, height))
}
