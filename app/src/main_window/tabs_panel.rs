use gpui::{Context, Div, ParentElement, StatefulInteractiveElement, Styled, div, px, rgb};
use ui::Theme;

use super::MainWindow;

const MIN_TAB_SPACER_WIDTH: f32 = 24.0;

/// Draws one tab group: tab strip, menu button, and the panel content surface.
pub(crate) struct TabsPanel {
    theme: Theme,
    active_tab: usize,
}

impl TabsPanel {
    pub(crate) fn new(theme: Theme) -> Self {
        Self {
            theme,
            active_tab: 0,
        }
    }

    pub(crate) fn render(&self, cx: &mut Context<MainWindow>, height: f32) -> Div {
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
        let content_height = (height - strip_height).max(0.0);

        let first_tab_background = if self.active_tab == 0 {
            colors.active_tab
        } else {
            colors.toolbar_background
        };
        let first_tab_text = if self.active_tab == 0 {
            colors.primary_text
        } else {
            colors.muted_text
        };
        let second_tab_background = if self.active_tab == 1 {
            colors.active_tab
        } else {
            colors.toolbar_background
        };
        let second_tab_text = if self.active_tab == 1 {
            colors.primary_text
        } else {
            colors.muted_text
        };

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
                    .bg(rgb(first_tab_background))
                    .text_color(rgb(first_tab_text))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .id("tabs-panel-tab-layers")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tabs_panel.active_tab = 0;
                        cx.notify();
                    }))
                    .child("Layers"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(tab_width))
                    .h(px(strip_height))
                    .bg(rgb(second_tab_background))
                    .text_color(rgb(second_tab_text))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .id("tabs-panel-tab-color")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tabs_panel.active_tab = 1;
                        cx.notify();
                    }))
                    .child("Color"),
            )
            // Intentionally empty space between the tabs and the menu button.
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

        // The content surface fills all remaining height so no empty strip is left below it.
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
            .bg(rgb(colors.group_background))
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

    #[test]
    fn content_occupies_all_height_below_the_tab_strip() {
        let theme = Theme::default();
        let height = 700.0;
        let strip_height = theme.metrics.tab_strip_height.min(height).max(1.0);
        let content_height = (height - strip_height).max(0.0);

        assert!((strip_height + content_height - height).abs() < 0.001);
    }
}
