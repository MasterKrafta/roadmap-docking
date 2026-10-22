use gpui::{Context, IntoElement, Render, Styled, Window, div, px, rgb};
use ui::Theme;

/// Minimal application window containing one empty docking column.
///
/// The column has three independent rows:
/// 1. Header: empty space on the left and the two requested controls on the right.
/// 2. Tabs strip: empty tab area and a menu control on the right.
/// 3. Content: an empty area that will be filled in later.
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
        let size = window.viewport_size();
        let width = size.width.as_f32().max(1.0);
        let height = size.height.as_f32().max(1.0);
        let colors = self.theme.colors;

        let control = |label: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(28.0))
                .h(px(28.0))
                .text_color(rgb(colors.muted_text))
                .child(label)
        };

        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.window_background))
            // 1. Header: empty space on the left, controls on the right.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(32.0))
                    .flex_shrink_0()
                    .bg(rgb(colors.group_background))
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(div().flex_1())
                    .child(control(">>"))
                    .child(control("x")),
            )
            // 2. Tabs strip: an empty tab area with the hamburger at the right.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(32.0))
                    .flex_shrink_0()
                    .bg(rgb(colors.dock_background))
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(div().flex_1().h(px(32.0)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(32.0))
                            .h(px(32.0))
                            .text_color(rgb(colors.muted_text))
                            .child("☰"),
                    ),
            )
            // 3. Content: deliberately blank for now.
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .bg(rgb(colors.canvas_background)),
            )
    }
}
