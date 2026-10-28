use dock_core::Rect;
use gpui::{
    App, InteractiveElement, IntoElement, ParentElement, RenderOnce, StatefulInteractiveElement,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::{ClickHandler, with_focus_ring};

/// One rendered row in the popup menu.
pub struct PopupMenuItem {
    pub id: String,
    pub label: String,
    pub checked: bool,
    pub enabled: bool,
    pub selected: bool,
    pub on_select: Option<ClickHandler>,
}

/// A bounded popup rendered above the dock surface.
#[derive(IntoElement)]
pub struct PopupMenu {
    bounds: Rect,
    theme: Theme,
    items: Vec<PopupMenuItem>,
}

impl PopupMenu {
    pub fn new(bounds: Rect, theme: &Theme, items: Vec<PopupMenuItem>) -> Self {
        Self {
            bounds,
            theme: *theme,
            items,
        }
    }
}

impl RenderOnce for PopupMenu {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let typography = self.theme.typography;
        let mut popup = div()
            .absolute()
            .left(px(self.bounds.origin.x))
            .top(px(self.bounds.origin.y))
            .w(px(self.bounds.size.width))
            .h(px(self.bounds.size.height))
            .bg(rgb(self.theme.colors.popup_menu_background))
            .text_color(rgb(self.theme.colors.primary_text))
            .font_family(typography.family)
            .text_size(px(typography.body_size))
            .line_height(px(typography.line_height))
            .border_1()
            .border_color(rgb(self.theme.colors.popup_menu_border));

        for item in self.items {
            let mut row = div()
                .id(item.id.clone())
                .flex()
                .items_center()
                .w(px(self.bounds.size.width))
                .h(px(self.theme.metrics.popup_menu_row_height))
                .px(px(self.theme.metrics.popup_menu_horizontal_padding))
                .text_color(rgb(self.theme.colors.primary_text));

            if item.selected {
                row = row.bg(rgb(self.theme.colors.popup_menu_selected));
            }

            row = row.child(if item.checked { "✓" } else { " " });
            row = row.child(item.label);

            if item.enabled
                && let Some(on_select) = item.on_select
            {
                let hover_color = self.theme.colors.popup_menu_hover;
                popup = popup.child(
                    with_focus_ring(row, &self.theme)
                        .hover(move |style| style.bg(rgb(hover_color)))
                        .on_click(on_select),
                );
                continue;
            }
            popup = popup.child(row);
        }

        popup
    }
}
