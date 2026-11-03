use gpui::{IntoElement, InteractiveElement, Styled, div, rgb};
use ui::Theme;

/// Flexible drag surface for the whole Column header.
///
/// DockHost's shared pointer pipeline recognises this Core header region as a
/// ColumnHeader drag. It floats the complete Column through DockCommand::Float;
/// FloatingWindowManager then hosts that composition in a WindowKind::Floating window.
pub struct ColumnDragArea;

impl ColumnDragArea {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .min_w_0()
            .h_full()
            .cursor_move()
            .bg(rgb(theme.colors.group_grip_background))
    }
}
