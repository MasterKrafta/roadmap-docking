use dock_core::NodeId;
use gpui::{IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

use crate::components::context_menu::{ContextMenuTarget, GroupMenuToggle};

/// Fixed-width menu control for the Group.
pub struct GroupMenuButton;

impl GroupMenuButton {
    pub fn render(id: String, theme: Theme) -> impl IntoElement {
        Self::render_with_metrics(id, theme, 32.0, 14.0)
    }

    pub fn render_with_metrics(
        id: String,
        theme: Theme,
        width: f32,
        icon_size: f32,
    ) -> impl IntoElement {
        Self::render_with_glyph(id, theme, width, icon_size, "☰")
    }

    /// Renders the runtime Group menu button and forwards its viewport-local
    /// pointer anchor to the host-owned ContextMenuToggle callback.
    pub(crate) fn render_for_group(
        id: String,
        group: NodeId,
        theme: Theme,
        width: f32,
        icon_size: f32,
        glyph: &'static str,
        toggle_menu: GroupMenuToggle,
    ) -> impl IntoElement {
        Self::render_with_glyph(id, theme, width, icon_size, glyph)
            .on_click(move |event, _, app| {
                let Ok(anchor) = crate::geometry::point_from_gpui(event.position()) else {
                    return;
                };
                (toggle_menu)(ContextMenuTarget::Group(group), anchor, app);
            })
    }

    fn render_with_glyph(
        id: String,
        theme: Theme,
        width: f32,
        icon_size: f32,
        glyph: &'static str,
    ) -> impl IntoElement + InteractiveElement {
        div()
            .id(id)
            .w(px(width))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.group_menu_button))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(icon_size))
            .child(glyph)
    }
}
