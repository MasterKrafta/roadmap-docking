use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Window, div, px, rgb,
};
use ui::Theme;

/// Minimal application window containing one empty docking column with a two-tab group.
pub(crate) struct MainWindow {
    theme: Theme,
    active_tab: usize,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        Ok(Self {
            theme,
            active_tab: 0,
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        let first_tab_background = if self.active_tab == 0 {
            colors.active_tab
        } else {
            colors.dock_background
        };
        let first_tab_text = if self.active_tab == 0 {
            colors.primary_text
        } else {
            colors.muted_text
        };
        let second_tab_background = if self.active_tab == 1 {
            colors.active_tab
        } else {
            colors.dock_background
        };
        let second_tab_text = if self.active_tab == 1 {
            colors.primary_text
        } else {
            colors.muted_text
        };

        // Fill the entire window with the dark theme so the area beside the
        // 280 px column does not fall back to the platform's white background.
        div()
            .flex()
            .flex_row()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.window_background))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(280.0))
                    .min_w(px(200.0))
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
                    // 2. Tabs strip: two switchable tabs, free space, and the menu on the right.
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
                            .child(
                                div()
                                    .id("main-window-tab-1")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(104.0))
                                    .h(px(32.0))
                                    .bg(rgb(first_tab_background))
                                    .text_color(rgb(first_tab_text))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.active_tab = 0;
                                        cx.notify();
                                    }))
                                    .child("Вкладка 1"),
                            )
                            .child(
                                div()
                                    .id("main-window-tab-2")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(104.0))
                                    .h(px(32.0))
                                    .bg(rgb(second_tab_background))
                                    .text_color(rgb(second_tab_text))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.active_tab = 1;
                                        cx.notify();
                                    }))
                                    .child("Вкладка 2"),
                            )
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
                    ),
            )
    }
}
