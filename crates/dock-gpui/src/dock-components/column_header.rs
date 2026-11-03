use super::{CollapseColumnButton, CloseColumnButton};
use dock_core::DockMetrics;
use gpui::{IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

/// Fixed visual header that belongs to the Column rather than to a Group.
pub struct ColumnHeader;

impl ColumnHeader {
    pub fn render(id: String, metrics: DockMetrics, theme: Theme) -> impl IntoElement {
        let mut header = div()
            .id(format!("{id}-header"))
            .w_full()
            .h(px(metrics.column_header_height))
            .flex_none()
            .flex()
            .items_stretch()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(rgb(theme.colors.group_grip_background)),
            );

        // These controls are visual only; their actions are not connected yet.
        header = header
            .child(CollapseColumnButton::render_with_metrics(
                format!("{id}-collapse"),
                theme,
                metrics.column_control_button_width,
                metrics.control_icon_size,
            ))
            .child(CloseColumnButton::render_with_metrics(
                format!("{id}-close"),
                theme,
                metrics.column_control_button_width,
                metrics.control_icon_size,
            ));

        header
    }
}
