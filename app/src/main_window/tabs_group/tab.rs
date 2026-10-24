use gpui::{
    App, AppContext, ClickEvent, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

use super::super::{TabDragPayload, TabDragPreview};

/// A selectable tab. The parent owns the active-tab state and click callback.
pub(super) fn render<F>(
    theme: ui::Theme,
    label: &'static str,
    is_active: bool,
    height: f32,
    on_click: F,
    drag_payload: Option<TabDragPayload>,
) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let background = if is_active {
        theme.colors.active_tab
    } else {
        theme.colors.toolbar_background
    };

    let tab = div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(theme.metrics.column_tab_width.max(1.0)))
        .h(px(height))
        .bg(rgb(background))
        .text_color(rgb(theme.colors.primary_text))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(label)
        .id(label)
        .on_click(on_click);

    match drag_payload {
        Some(payload) => tab.on_drag(payload, |_, _, _, cx| cx.new(|_| TabDragPreview)),
        None => tab,
    }
}
