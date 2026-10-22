use gpui::{
    Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
    rgb,
};
use ui::Theme;

/// Minimal application window containing one right-docked column with two stacked tab groups.
pub(crate) struct MainWindow {
    theme: Theme,
    active_tab: usize,
    lower_active_tab: usize,
    column_width: f32,
    resize_start_x: f32,
    resize_start_y: f32,
    resize_start_width: f32,
    resize_start_top_height: f32,
    split_ratio: f32,
    is_resizing: bool,
    is_resizing_height: bool,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let column_width = theme.metrics.default_column_width;
        let split_ratio = theme.metrics.default_column_split_ratio;

        Ok(Self {
            theme,
            active_tab: 0,
            lower_active_tab: 0,
            column_width,
            resize_start_x: 0.0,
            resize_start_y: 0.0,
            resize_start_width: column_width,
            resize_start_top_height: 0.0,
            split_ratio,
            is_resizing: false,
            is_resizing_height: false,
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
        let lower_first_tab_background = if self.lower_active_tab == 0 {
            colors.active_tab
        } else {
            colors.dock_background
        };
        let lower_first_tab_text = if self.lower_active_tab == 0 {
            colors.primary_text
        } else {
            colors.muted_text
        };
        let lower_second_tab_background = if self.lower_active_tab == 1 {
            colors.active_tab
        } else {
            colors.dock_background
        };
        let lower_second_tab_text = if self.lower_active_tab == 1 {
            colors.primary_text
        } else {
            colors.muted_text
        };

        let vertical_splitter_color = if self.is_resizing {
            colors.focus
        } else {
            colors.splitter
        };
        let horizontal_splitter_color = if self.is_resizing_height {
            colors.focus
        } else {
            colors.splitter
        };

        // The root fills the window. While dragging a splitter, keep its resize
        // cursor even when the pointer travels outside the narrow handle.
        let root = div()
            .flex()
            .flex_row()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.window_background));
        let root = if self.is_resizing {
            root.cursor(CursorStyle::ResizeLeftRight)
        } else if self.is_resizing_height {
            root.cursor(CursorStyle::ResizeUpDown)
        } else {
            root
        };

        root
            .on_mouse_move({
                let min_width = metrics.min_column_width;
                let hit_zone = metrics.splitter_hit_zone;
                let header_height = metrics.column_header_height;
                let splitter_height = metrics.splitter_hit_zone;
                let min_group_height = metrics.min_group_height;
                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    if this.is_resizing {
                        let window_width = window.viewport_size().width.as_f32().max(1.0);
                        let max_width = (window_width - hit_zone).max(min_width);
                        let delta = this.resize_start_x - event.position.x.as_f32();
                        this.column_width =
                            (this.resize_start_width + delta).clamp(min_width, max_width);
                        cx.notify();
                    } else if this.is_resizing_height {
                        let window_height = window.viewport_size().height.as_f32().max(1.0);
                        let actual_header_height = header_height.min(window_height);
                        let actual_splitter_height =
                            splitter_height.min((window_height - actual_header_height).max(0.0));
                        let available_height = (window_height
                            - actual_header_height
                            - actual_splitter_height)
                            .max(0.0);
                        let min_height = min_group_height.min(available_height / 2.0);
                        let max_height = (available_height - min_height).max(min_height);
                        let delta = event.position.y.as_f32() - this.resize_start_y;
                        let upper_height = (this.resize_start_top_height + delta)
                            .clamp(min_height, max_height);
                        this.split_ratio = if available_height > 0.0 {
                            upper_height / available_height
                        } else {
                            0.5
                        };
                        cx.notify();
                    }
                })
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    if this.is_resizing || this.is_resizing_height {
                        this.is_resizing = false;
                        this.is_resizing_height = false;
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
            // Width splitter on the column's unattached (left) edge.
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(metrics.splitter_hit_zone))
                    .h(px(height))
                    .flex_shrink_0()
                    .bg(rgb(colors.window_background))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                            this.column_width = column_width;
                            this.is_resizing = true;
                            this.is_resizing_height = false;
                            this.resize_start_x = event.position.x.as_f32();
                            this.resize_start_width = column_width;
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(metrics.splitter_thickness))
                            .h(px(height))
                            .bg(rgb(vertical_splitter_color)),
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
                    // Shared column header.
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .h(px(column_header_height))
                            .flex_shrink_0()
                            .bg(rgb(colors.group_background))
                            .border_b_1()
                            .border_color(rgb(colors.border))
                            .child(div().flex_1())
                            .child(control(">>"))
                            .child(control("x")),
                    )
                    // Upper panel: tabs plus an empty content area.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .h(px(top_panel_height))
                            .flex_shrink_0()
                            .min_h(px(0.0))
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
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .bg(rgb(colors.canvas_background)),
                            ),
                    )
                    // Horizontal splitter separates the upper and lower tab groups.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(column_width))
                            .h(px(horizontal_splitter_height))
                            .flex_shrink_0()
                            .bg(rgb(colors.window_background))
                            .cursor(CursorStyle::ResizeUpDown)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                                    this.is_resizing = false;
                                    this.is_resizing_height = true;
                                    this.resize_start_y = event.position.y.as_f32();
                                    this.resize_start_top_height = top_panel_height;
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .w(px(column_width))
                                    .h(px(metrics.splitter_thickness.min(horizontal_splitter_height)))
                                    .bg(rgb(horizontal_splitter_color)),
                            ),
                    )
                    // Lower panel: its own independent tabs plus an empty content area.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .h(px(bottom_panel_height))
                            .flex_shrink_0()
                            .min_h(px(0.0))
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
                                            .id("main-window-bottom-tab-1")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .w(px(tab_width))
                                            .h(px(metrics.tab_strip_height))
                                            .bg(rgb(lower_first_tab_background))
                                            .text_color(rgb(lower_first_tab_text))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.lower_active_tab = 0;
                                                cx.notify();
                                            }))
                                            .child("Вкладка 1"),
                                    )
                                    .child(
                                        div()
                                            .id("main-window-bottom-tab-2")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .w(px(tab_width))
                                            .h(px(metrics.tab_strip_height))
                                            .bg(rgb(lower_second_tab_background))
                                            .text_color(rgb(lower_second_tab_text))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.lower_active_tab = 1;
                                                cx.notify();
                                            }))
                                            .child("Вкладка 2"),
                                    )
                                    .child(div().flex_1().h(px(metrics.tab_strip_height)))
                                    .child(control("☰")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .bg(rgb(colors.canvas_background)),
                            ),
                    ),
            )
    }
}
