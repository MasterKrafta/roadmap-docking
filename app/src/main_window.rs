use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use ui::Theme;

/// Main application window with a minimal right-side docking column.
pub(crate) struct MainWindow {
    theme: Theme,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        Ok(Self { theme })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let window_width = viewport.width.as_f32().max(1.0);
        let window_height = viewport.height.as_f32().max(1.0);

        let colors = self.theme.colors;
        let metrics = self.theme.metrics;

        let column_width = metrics.default_column_width.min(window_width);
        let workspace_width = (window_width - column_width).max(0.0);
        let header_height = metrics.column_header_height.min(window_height);
        let tab_strip_height = metrics
            .tab_strip_height
            .min((window_height - header_height).max(0.0));
        let content_height = (window_height - header_height - tab_strip_height).max(0.0);

        let button_size = metrics.button_size;
        let button_gap = metrics.button_gap;
        let content_padding = metrics.content_padding.min(column_width / 2.0);

        let header_spacer_width =
            (column_width - content_padding - button_size * 2.0 - button_gap).max(0.0);

        let tab_width = metrics
            .column_tab_width
            .min((column_width - content_padding * 2.0 - button_size).max(0.0));
        let tab_spacer_width =
            (column_width - content_padding * 2.0 - tab_width - button_size).max(0.0);

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_width))
            .h(px(header_height))
            .bg(rgb(colors.toolbar_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_size))
                    .h(px(button_size))
                    .bg(rgb(colors.hover))
                    .text_color(rgb(colors.primary_text))
                    .child(">>"),
            )
            .child(div().w(px(button_gap)).h(px(1.0)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_size))
                    .h(px(button_size))
                    .bg(rgb(colors.hover))
                    .text_color(rgb(colors.primary_text))
                    .child("×"),
            )
            .child(div().w(px(header_spacer_width)).h(px(1.0)));

        let tab_strip = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(column_width))
            .h(px(tab_strip_height))
            .bg(rgb(colors.dock_background))
            .border_1()
            .border_color(rgb(colors.border))
            .child(div().w(px(content_padding)).h(px(1.0)))
            .child(
                div()
                    .w(px(tab_width))
                    .h(px(tab_strip_height))
                    .bg(rgb(colors.active_tab))
                    .border_1()
                    .border_color(rgb(colors.border)),
            )
            .child(div().w(px(tab_spacer_width)).h(px(1.0)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(button_size))
                    .h(px(button_size))
                    .text_color(rgb(colors.primary_text))
                    .bg(rgb(colors.hover))
                    .child("☰"),
            )
            .child(div().w(px(content_padding)).h(px(1.0)));

        let content = div()
            .w(px(column_width))
            .h(px(content_height))
            .bg(rgb(colors.group_background))
            .border_1()
            .border_color(rgb(colors.border));

        let right_column = div()
            .flex()
            .flex_col()
            .w(px(column_width))
            .h(px(window_height))
            .bg(rgb(colors.dock_background))
            .child(header)
            .child(tab_strip)
            .child(content);

        div()
            .flex()
            .flex_row()
            .w(px(window_width))
            .h(px(window_height))
            .bg(rgb(colors.canvas_background))
            .child(div().w(px(workspace_width)).h(px(window_height)))
            .child(right_column)
    }
}
