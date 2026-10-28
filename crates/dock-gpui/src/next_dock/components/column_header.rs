//! Empty drag area and controls for a dock column.

use dock_core::{Point, Rect, Size};
use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, rgb};
use ui::Theme;

use crate::components::{ClickHandler, IconButton};

#[derive(IntoElement)]
pub(in crate::next_dock) struct ColumnHeader {
    pub(in crate::next_dock) rect: Rect,
    pub(in crate::next_dock) theme: Theme,
    pub(in crate::next_dock) key: String,
}

impl RenderOnce for ColumnHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let rect = self.rect;
        let theme = self.theme;
        let width = rect.size.width.max(0.0);
        let height = rect.size.height.max(0.0);

        let button_gap = theme.metrics.button_gap.max(0.0).min(width);
        let button_width = theme
            .metrics
            .button_size
            .max(0.0)
            .min(height)
            .min(((width - button_gap).max(0.0)) / 2.0);

        let collapse_x = (width - button_width * 2.0 - button_gap).max(0.0);
        let close_x = (width - button_width).max(0.0);

        let collapse_rect = Rect::new(Point::new(collapse_x, 0.0), Size::new(button_width, height));

        let close_rect = Rect::new(Point::new(close_x, 0.0), Size::new(button_width, height));

        let background = theme.colors.toolbar_background;
        let hover_background = theme.colors.icon_button_hover;

        let header = div()
            .absolute()
            .left(px(rect.origin.x))
            .top(px(rect.origin.y))
            .w(px(width))
            .h(px(height))
            .bg(rgb(background))
            .border_1()
            .border_color(rgb(theme.colors.border));

        header
            .child(
                IconButton::new(
                    format!("next-dock-column-collapse-{}", self.key),
                    collapse_rect,
                    ">>",
                    &theme,
                    noop_click(),
                )
                .with_colors(background, hover_background)
                .with_border()
                .with_centered_glyph()
                .into_any_element(),
            )
            .child(
                IconButton::new(
                    format!("next-dock-column-close-{}", self.key),
                    close_rect,
                    "×",
                    &theme,
                    noop_click(),
                )
                .with_colors(background, hover_background)
                .with_border()
                .with_centered_glyph()
                .into_any_element(),
            )
    }
}

fn noop_click() -> ClickHandler {
    Box::new(|_, _, _| {})
}
