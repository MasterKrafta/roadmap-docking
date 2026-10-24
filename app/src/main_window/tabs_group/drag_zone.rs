use gpui::{Div, ParentElement, Styled, div, px, rgb};

/// The free area after the tabs, including a reserved strip for future drag/drop targeting.
pub(super) fn render(theme: ui::Theme, height: f32) -> Div {
    div()
        .flex_1()
        .h(px(height))
        .bg(rgb(theme.colors.toolbar_background))
        .child(
            div()
                .flex()
                .w(px(theme.metrics.tab_drag_zone_width.max(0.0)))
                .h(px(height))
                .bg(rgb(theme.colors.dock_background)),
        )
}
