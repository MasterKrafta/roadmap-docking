use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use ui::Theme;

/// Minimal application window containing one empty docking column with a two-tab group.
pub(crate) struct MainWindow {
    theme: Theme,
    active_tab: usize,
    column_width: f32,
    resize_start_x: f32,
    resize_start_width: f32,
    is_resizing: bool,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let column_width = theme.metrics.default_column_width;

        Ok(Self {
            theme,
            active_tab: 0,
            column_width,
            resize_start_x: 0.0,
            resize_start_width: column_width,
            is_resizing: false,
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let width = size.width.as_f32().max(1.0);
        let height = size.height.as_f32().max(1.0);
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;

        let max_column_width = (width - metrics.splitter_hit_zone).max(metrics.min_column_width);
        let column_width = self
            .column_width
            .clamp(metrics.min_column_width, max_column_width);
        let tab_width =
            metrics.column_tab_width.min(((column_width - metrics.button_size).max(0.0)) / 2.0);

        let control = |label: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(metrics.button_size))
                .h(px(metrics.button_size))
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
        let splitter_color = if self.is_resizing {
            colors.focus
        } else {
            colors.splitter
        };

        // The root fills the entire window; the dock column is attached to the right.
        div()
            .flex()
            .flex_row()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.window_background))
            .on_mouse_move({
                let min_width = metrics.min_column_width;
                let hit_zone = metrics.splitter_hit_zone;
                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    if !this.is_resizing {
                        return;
                    }

                    let window_width = window.viewport_size().width.as_f32().max(1.0);
                    let max_width = (window_width - hit_zone).max(min_width);
                    let delta = this.resize_start_x - event.position.x.as_f32();
                    this.column_width =
                        (this.resize_start_width + delta).clamp(min_width, max_width);
                    cx.notify();
                })
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    if this.is_resizing {
                        this.is_resizing = false;
                        cx.notify();
                    }
                }),
            )
            .child(
                // Empty workspace to the left of the dock column.
                div()
                    .flex_1()
                    .h(px(height))
                    .bg(rgb(colors.window_background)),
            )
            // Resizer lives on the column's unattached (left) edge.
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(metrics.splitter_hit_zone))
                    .h(px(height))
                    .flex_shrink_0()
                    .bg(rgb(colors.window_background))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, _window, cx| {
                            // Start from the displayed width, which may already be clamped
                            // if the window was resized to a narrower size.
                            this.column_width = column_width;
                            this.is_resizing = true;
                            this.resize_start_x = event.position.x.as_f32();
                            this.resize_start_width = column_width;
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(metrics.splitter_thickness))
                            .h(px(height))
                            .bg(rgb(splitter_color)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(column_width))
                    .min_w(px(metrics.min_column_width))
                    .h(px(height))
                    .flex_shrink_0()
                    .bg(rgb(colors.window_background))
                    // 1. Header: empty space on the left, controls on the right.
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .h(px(metrics.column_header_height))
                            .flex_shrink_0()
                            .bg(rgb(colors.group_background))
                            .border_b_1()
                            .border_color(rgb(colors.border))
                            .child(div().flex_1())
                            .child(control(">>"))
                            .child(control("x")),
                    )
                    // 2. Tab strip: two switchable tabs, empty space, and the menu on the right.
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .h(px(metrics.tab_strip_height))
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
                                    .w(px(tab_width))
                                    .h(px(metrics.tab_strip_height))
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
                                    .w(px(tab_width))
                                    .h(px(metrics.tab_strip_height))
                                    .bg(rgb(second_tab_background))
                                    .text_color(rgb(second_tab_text))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.active_tab = 1;
                                        cx.notify();
                                    }))
                                    .child("Вкладка 2"),
                            )
                            .child(div().flex_1().h(px(metrics.tab_strip_height)))
                            .child(control("☰")),
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
