use dock_core::{ColumnMode, Rect};
use gpui::{
    App, ClickEvent, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::components::with_focus_ring;

/// A panel entry in a flattened dock rail.
#[derive(IntoElement)]
pub struct RailEntry {
    /// Stable identity supplied by the renderer.
    pub id: String,
    /// Absolute bounds, clipped to the visible rail viewport if needed.
    pub rect: Rect,
    /// Presentation mode controls whether icon, title, or both are shown.
    pub mode: ColumnMode,
    /// Panel icon/glyph.
    pub icon: String,
    /// Panel title.
    pub title: String,
    /// Whether this panel is active or selected by the flyout.
    pub active: bool,
    /// Shared application theme.
    pub theme: Theme,
    /// Requests opening this panel in the flyout.
    pub on_open: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
}

impl RenderOnce for RailEntry {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let label = crate::labels::rail_entry_label(self.mode, &self.icon, &self.title);
        let background = if self.active {
            self.theme.colors.active_tab
        } else {
            self.theme.colors.toolbar_background
        };
        let hover_background = if self.active {
            background
        } else {
            self.theme.colors.hover
        };
        let typography = self.theme.typography;
        let entry = div()
            .absolute()
            .left(px(self.rect.origin.x))
            .top(px(self.rect.origin.y))
            .w(px(self.rect.size.width))
            .h(px(self.rect.size.height))
            .bg(rgb(background))
            .text_color(rgb(self.theme.colors.primary_text))
            .font_family(typography.family)
            .text_size(px(typography.tab_size))
            .line_height(px(typography.line_height));
        let entry = if self.mode == ColumnMode::IconsOnly {
            entry.flex().items_center().justify_center().child(label)
        } else {
            entry.flex().items_center().child(label)
        };

        with_focus_ring(entry.id(self.id), &self.theme)
            .hover(move |style| style.bg(rgb(hover_background)))
            .on_click(self.on_open)
    }
}
