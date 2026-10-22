use gpui::{
    Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
    rgb,
};
use ui::Theme;

/// Two right-docked columns. Each column has its own top and bottom tab groups.
pub(crate) struct MainWindow {
    theme: Theme,
    active_tabs: [usize; 2],
    lower_active_tabs: [usize; 2],
    column_widths: [f32; 2],
    resize_start_x: f32,
    resize_start_y: f32,
    resize_start_column_widths: [f32; 2],
    resize_start_top_height: f32,
    split_ratios: [f32; 2],
    resizing_height_column: usize,
    is_resizing_columns: bool,
    is_resizing_height: bool,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let default_width = theme.metrics.default_column_width;
        let default_split = theme.metrics.default_column_split_ratio;

        Ok(Self {
            theme,
            active_tabs: [0, 0],
            lower_active_tabs: [0, 0],
            column_widths: [default_width, default_width],
            resize_start_x: 0.0,
            resize_start_y: 0.0,
            resize_start_column_widths: [default_width, default_width],
            resize_start_top_height: 0.0,
            split_ratios: [default_split, default_split],
            resizing_height_column: 0,
            is_resizing_columns: false,
            is_resizing_height: false,
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let window_width = size.width.as_f32().max(1.0);
        let window_height = size.height.as_f32().max(1.0);
        let colors = self.theme.colors;
        let metrics = self.theme.metrics;

        let min_column_width = metrics.min_column_width;
        let divider_width = metrics.splitter_hit_zone;
        let max_pair_width = (window_width - divider_width)
            .max(min_column_width * 2.0);
        let pair_width = (self.column_widths[0] + self.column_widths[1])
            .min(max_pair_width)
            .max(min_column_width * 2.0);
        let left_column_width = self.column_widths[0]
            .clamp(min_column_width, pair_width - min_column_width);
        let right_column_width = pair_width - left_column_width;

        let header_height = metrics.column_header_height.min(window_height);
        let horizontal_divider_height =
            metrics.splitter_hit_zone.min((window_height - header_height).max(0.0));
        let available_panel_height =
            (window_height - header_height - horizontal_divider_height).max(0.0);
        let min_panel_height = metrics.min_group_height.min(available_panel_height / 2.0);

        let panel_heights = |ratio: f32| {
            let max_top_height =
                (available_panel_height - min_panel_height).max(min_panel_height);
            let top = (available_panel_height * ratio)
                .clamp(min_panel_height, max_top_height);
            (top, (available_panel_height - top).max(0.0))
        };

        let (left_top_height, left_bottom_height) = panel_heights(self.split_ratios[0]);
        let (right_top_height, right_bottom_height) = panel_heights(self.split_ratios[1]);

        let vertical_splitter_color = if self.is_resizing_columns {
            colors.focus
        } else {
            colors.splitter
        };
        let root = div()
            .flex()
            .flex_row()
            .w(px(window_width))
            .h(px(window_height))
            .bg(rgb(colors.window_background));

        let root = if self.is_resizing_columns {
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
                let min_group_height = metrics.min_group_height;
                let column_header_height = metrics.column_header_height;
                let default_split = metrics.default_column_split_ratio;

                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    if this.is_resizing_columns {
                        let actual_window_width =
                            window.viewport_size().width.as_f32().max(1.0);
                        let max_total_width =
                            (actual_window_width - hit_zone).max(min_width * 2.0);
                        let start_total_width = this.resize_start_column_widths[0]
                            + this.resize_start_column_widths[1];
                        let total_width = start_total_width
                            .min(max_total_width)
                            .max(min_width * 2.0);
                        let delta = event.position.x.as_f32() - this.resize_start_x;
                        let left_width = (this.resize_start_column_widths[0] + delta)
                            .clamp(min_width, total_width - min_width);

                        this.column_widths = [left_width, total_width - left_width];
                        cx.notify();
                    } else if this.is_resizing_height {
                        let actual_window_height =
                            window.viewport_size().height.as_f32().max(1.0);
                        let actual_header_height =
                            column_header_height.min(actual_window_height);
                        let actual_divider_height = hit_zone
                            .min((actual_window_height - actual_header_height).max(0.0));
                        let total_panel_height = (actual_window_height
                            - actual_header_height
                            - actual_divider_height)
                            .max(0.0);
                        let minimum = min_group_height.min(total_panel_height / 2.0);
                        let maximum = (total_panel_height - minimum).max(minimum);
                        let delta = event.position.y.as_f32() - this.resize_start_y;
                        let top_height = (this.resize_start_top_height + delta)
                            .clamp(minimum, maximum);
                        let index = this.resizing_height_column.min(1);

                        this.split_ratios[index] = if total_panel_height > 0.0 {
                            top_height / total_panel_height
                        } else {
                            default_split
                        };
                        cx.notify();
                    }
                })
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    if this.is_resizing_columns || this.is_resizing_height {
                        this.is_resizing_columns = false;
                        this.is_resizing_height = false;
                        cx.notify();
                    }
                }),
            )
            // Empty workspace remains to the left of both docked columns.
            .child(
                div()
                    .flex_1()
                    .h(px(window_height))
                    .bg(rgb(colors.window_background)),
            )
            .child(render_column(
                0,
                left_column_width,
                left_top_height,
                left_bottom_height,
                self.theme,
                self.active_tabs[0],
                self.lower_active_tabs[0],
                self.split_ratios[0],
                horizontal_splitter_height,
                self.is_resizing_height && self.resizing_height_column == 0,
                cx,
            ))
            // Shared vertical splitter: dragging it reallocates width between both columns.
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(divider_width))
                    .h(px(window_height))
                    .flex_shrink_0()
                    .bg(rgb(colors.window_background))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                            this.column_widths = [left_column_width, right_column_width];
                            this.is_resizing_columns = true;
                            this.is_resizing_height = false;
                            this.resize_start_x = event.position.x.as_f32();
                            this.resize_start_column_widths =
                                [left_column_width, right_column_width];
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(metrics.splitter_thickness))
                            .h(px(window_height))
                            .bg(rgb(vertical_splitter_color)),
                    ),
            )
            .child(render_column(
                1,
                right_column_width,
                right_top_height,
                right_bottom_height,
                self.theme,
                self.active_tabs[1],
                self.lower_active_tabs[1],
                self.split_ratios[1],
                horizontal_splitter_height,
                self.is_resizing_height && self.resizing_height_column == 1,
                cx,
            ))
    }
}

fn render_column(
    column_index: usize,
    column_width: f32,
    top_panel_height: f32,
    bottom_panel_height: f32,
    theme: Theme,
    active_tab: usize,
    lower_active_tab: usize,
    split_ratio: f32,
    horizontal_splitter_height: f32,
    is_resizing_height: bool,
    cx: &mut Context<MainWindow>,
) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let horizontal_splitter_color = if is_resizing_height {
        colors.focus
    } else {
        colors.splitter
    };
    let tab_width =
        metrics.column_tab_width.min(((column_width - metrics.button_size).max(0.0)) / 2.0);

    let (first_tab_id, second_tab_id, lower_first_tab_id, lower_second_tab_id) =
        if column_index == 0 {
            (
                "left-column-tab-1",
                "left-column-tab-2",
                "left-column-bottom-tab-1",
                "left-column-bottom-tab-2",
            )
        } else {
            (
                "right-column-tab-1",
                "right-column-tab-2",
                "right-column-bottom-tab-1",
                "right-column-bottom-tab-2",
            )
        };

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

    let first_tab_background = if active_tab == 0 {
        colors.active_tab
    } else {
        colors.dock_background
    };
    let first_tab_text = if active_tab == 0 {
        colors.primary_text
    } else {
        colors.muted_text
    };
    let second_tab_background = if active_tab == 1 {
        colors.active_tab
    } else {
        colors.dock_background
    };
    let second_tab_text = if active_tab == 1 {
        colors.primary_text
    } else {
        colors.muted_text
    };
    let lower_first_tab_background = if lower_active_tab == 0 {
        colors.active_tab
    } else {
        colors.dock_background
    };
    let lower_first_tab_text = if lower_active_tab == 0 {
        colors.primary_text
    } else {
        colors.muted_text
    };
    let lower_second_tab_background = if lower_active_tab == 1 {
        colors.active_tab
    } else {
        colors.dock_background
    };
    let lower_second_tab_text = if lower_active_tab == 1 {
        colors.primary_text
    } else {
        colors.muted_text
    };

    div()
        .flex()
        .flex_col()
        .w(px(column_width))
        .min_w(px(metrics.min_column_width))
        .h(px(
            // The requested layout fills the full window height.
            top_panel_height + horizontal_splitter_height + bottom_panel_height
                + metrics.column_header_height,
        ))
        .flex_shrink_0()
        .bg(rgb(colors.window_background))
        // Header with its controls aligned to the right.
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
        // Upper tab group and its content.
        .child(
            div()
                .flex()
                .flex_col()
                .h(px(top_panel_height))
                .flex_shrink_0()
                .min_h(px(0.0))
                .child(render_tab_strip(
                    column_index,
                    false,
                    column_width,
                    theme,
                    active_tab,
                    first_tab_id,
                    second_tab_id,
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .bg(rgb(colors.canvas_background)),
                ),
        )
        // Horizontal splitter is independently resizable in each column.
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
                        this.is_resizing_columns = false;
                        this.is_resizing_height = true;
                        this.resizing_height_column = column_index;
                        this.resize_start_y = event.position.y.as_f32();
                        this.resize_start_top_height = top_panel_height;
                        this.split_ratios[column_index] = split_ratio;
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
        // Lower tab group and its content.
        .child(
            div()
                .flex()
                .flex_col()
                .h(px(bottom_panel_height))
                .flex_shrink_0()
                .min_h(px(0.0))
                .child(render_tab_strip(
                    column_index,
                    true,
                    column_width,
                    theme,
                    lower_active_tab,
                    lower_first_tab_id,
                    lower_second_tab_id,
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .bg(rgb(colors.canvas_background)),
                ),
        )
}

fn render_tab_strip(
    column_index: usize,
    is_lower: bool,
    column_width: f32,
    theme: Theme,
    active_tab: usize,
    first_tab_id: &'static str,
    second_tab_id: &'static str,
    cx: &mut Context<MainWindow>,
) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let tab_width =
        metrics.column_tab_width.min(((column_width - metrics.button_size).max(0.0)) / 2.0);

    let first_tab_background = if active_tab == 0 {
        colors.active_tab
    } else {
        colors.dock_background
    };
    let first_tab_text = if active_tab == 0 {
        colors.primary_text
    } else {
        colors.muted_text
    };
    let second_tab_background = if active_tab == 1 {
        colors.active_tab
    } else {
        colors.dock_background
    };
    let second_tab_text = if active_tab == 1 {
        colors.primary_text
    } else {
        colors.muted_text
    };

    let activate_first = cx.listener(move |this, _, _, cx| {
        if is_lower {
            this.lower_active_tabs[column_index] = 0;
        } else {
            this.active_tabs[column_index] = 0;
        }
        cx.notify();
    });
    let activate_second = cx.listener(move |this, _, _, cx| {
        if is_lower {
            this.lower_active_tabs[column_index] = 1;
        } else {
            this.active_tabs[column_index] = 1;
        }
        cx.notify();
    });

    let control = div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(metrics.button_size))
        .h(px(metrics.button_size))
        .text_color(rgb(colors.muted_text))
        .child("☰");

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
                .id(first_tab_id)
                .flex()
                .items_center()
                .justify_center()
                .w(px(tab_width))
                .h(px(metrics.tab_strip_height))
                .bg(rgb(first_tab_background))
                .text_color(rgb(first_tab_text))
                .on_click(activate_first)
                .child("Вкладка 1"),
        )
        .child(
            div()
                .id(second_tab_id)
                .flex()
                .items_center()
                .justify_center()
                .w(px(tab_width))
                .h(px(metrics.tab_strip_height))
                .bg(rgb(second_tab_background))
                .text_color(rgb(second_tab_text))
                .on_click(activate_second)
                .child("Вкладка 2"),
        )
        .child(div().flex_1().h(px(metrics.tab_strip_height)))
        .child(control)
}
