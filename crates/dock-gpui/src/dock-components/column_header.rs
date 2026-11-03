use super::{CollapseColumnButton, CloseColumnButton, ColumnDragArea};
use dock_core::DockMetrics;
use gpui::{InteractiveElement, IntoElement, ParentElement, Styled, div, px};
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
            .child(ColumnDragArea::render(
                format!("{id}-drag"),
                theme,
            ));

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
