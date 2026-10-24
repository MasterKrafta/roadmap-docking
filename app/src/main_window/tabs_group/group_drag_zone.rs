use gpui::{
    Div, ParentElement, StatefulInteractiveElement, Styled, div, px,
    rgb,
};

use super::super::{DockDragPreview, GroupDragPayload};

/// The free area after the tabs is the drag handle for moving the whole group.
pub(super) fn render(
    theme: ui::Theme,
    height: f32,
    drag_payload: Option<GroupDragPayload>,
) -> Div {
    let zone = div()
        .flex_1()
        .justify_end()
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .child(
            div()
                .flex()
                .w(px(theme.metrics.tab_drag_zone_width.max(0.0)))
                .h(px(height))
                .bg(rgb(theme.colors.dock_background)),
        );

    match drag_payload {
        Some(payload) => zone.on_drag(payload, |payload, _cursor_offset, window, cx| {
            // Refresh the pointer origin at the actual drag start, just like tab dragging.
            payload.drag_start_position.set(window.mouse_position());
            cx.new(|_| DockDragPreview)
        }),
        None => zone,
    }
}
