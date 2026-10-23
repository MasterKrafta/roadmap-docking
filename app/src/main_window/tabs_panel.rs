use gpui::{Div, ParentElement, Styled, div, px, rgb};
use ui::Theme;

const MIN_TAB_SPACER_WIDTH: f32 = 24.0;

/// Draws one tab group: tab strip, menu button, and the panel content surface.
pub(crate) struct TabsPanel {
    theme: Theme,
}

impl TabsPanel {
    pub(crate) fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub(crate) fn render(&self, height: f32) -> Div {
        let metrics = self.theme.metrics;
        let colors = self.theme.colors;

        let width = metrics.default_column_width.max(1.0);
        let height = height.max(1.0);
        let strip_height = metrics.tab_strip_height.min(height).max(1.0);
        let button_width = metrics.button_size.min(width).max(1.0);
        let spacer_width = MIN_TAB_SPACER_WIDTH.min((width - button_width).max(0.0));
        let tabs_available_width = (width - button_width - spacer_width).max(0.0);
        let tab_width = metrics
            .column_tab_width
            .max(0.0)
            .min(tabs_available_width / 2.0);
        let actual_spacer_width = (width - button_width - tab_width * 2.0).max(spacer_width);
        let content_available_height = (height - strip_height).max(0.0);
        let content_height = content_available_height * 0.75;

        let strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(width))
            .h(px(strip_height))
            .bg(rgb(colors.toolbar_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(tab_width))
                    .h(px(strip_height))
                    .bg(rgb(colors.active_tab))
                    .text_color(rgb(colors.primary_text))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .child("Layers"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(tab_width))
                    .h(px(strip_height))
                    .bg(rgb(colors.toolbar_background))
                    .text_color(rgb(colors.muted_text))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .child("Color"),
            )
            // Intentionally empty space between the tabs and the menu button.
            // Its width is never smaller than the configured minimum where space allows.
            .child(div().w(px(actual_spacer_width)).h(px(strip_height)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_width))
                    .h(px(strip_height))
                    .bg(rgb(colors.toolbar_background))
                    .text_color(rgb(colors.primary_text))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .child("☰"),
            );

        let content = div()
            .w(px(width))
            .h(px(content_height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border));

        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(strip)
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::MIN_TAB_SPACER_WIDTH;
    use ui::Theme;

    #[test]
    fn default_width_leaves_space_between_two_tabs_and_menu() {
        let theme = Theme::default();
        let width = theme.metrics.default_column_width;
        let button_width = theme.metrics.button_size.min(width).max(1.0);
        let spacer_width = MIN_TAB_SPACER_WIDTH.min((width - button_width).max(0.0));
        let tabs_available = (width - button_width - spacer_width).max(0.0);
        let tab_width = theme.metrics.column_tab_width.max(0.0).min(tabs_available / 2.0);
        let actual_spacer = (width - button_width - tab_width * 2.0).max(spacer_width);

        assert!(actual_spacer >= MIN_TAB_SPACER_WIDTH);
        assert!(tab_width > 0.0);
        assert!(button_width > 0.0);
    }
}
