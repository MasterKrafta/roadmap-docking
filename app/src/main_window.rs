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
    visible_panels: [bool; 4],
    windows_menu_open: bool,
    resizing_height_column: usize,
    is_resizing_columns: bool,
    is_resizing_outer_splitter: bool,
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
            visible_panels: [true, true, true, true],
            windows_menu_open: false,
            resizing_height_column: 0,
            is_resizing_columns: false,
            is_resizing_outer_splitter: false,
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

        let menu_bar_height = 32.0_f32.min(window_height);
        let menu_item_height = 28.0;
        let menu_dropdown_height = if self.windows_menu_open {
            // Four 28px items, vertical padding, and three row gaps.
            menu_item_height * 4.0 + 28.0
        } else {
            0.0
        };
        let menu_area_height = (menu_bar_height + menu_dropdown_height).min(window_height - 1.0);
        let workspace_height = (window_height - menu_area_height).max(1.0);

        let min_column_width = metrics.min_column_width;
        let divider_width = metrics.splitter_hit_zone;
        // The dock group has two vertical handles: its outer edge and the inter-column splitter.
        let max_pair_width = (window_width - divider_width * 2.0)
            .max(min_column_width * 2.0);
        let pair_width = (self.column_widths[0] + self.column_widths[1])
            .min(max_pair_width)
            .max(min_column_width * 2.0);
        let left_column_width = self.column_widths[0]
            .clamp(min_column_width, pair_width - min_column_width);
        let right_column_width = pair_width - left_column_width;

        let header_height = metrics.column_header_height.min(workspace_height);
        let full_panel_height = (workspace_height - header_height).max(0.0);
        let horizontal_divider_height =
            metrics.splitter_hit_zone.min(full_panel_height);

        let panel_heights = |ratio: f32, show_top: bool, show_bottom: bool| {
            let divider = if show_top && show_bottom {
                horizontal_divider_height
            } else {
                0.0
            };
            let available = (full_panel_height - divider).max(0.0);

            if show_top && show_bottom {
                let min_height = metrics.min_group_height.min(available / 2.0);
                let max_top_height = (available - min_height).max(min_height);
                let top = (available * ratio).clamp(min_height, max_top_height);
                (top, (available - top).max(0.0), divider)
            } else if show_top {
                (available, 0.0, 0.0)
            } else if show_bottom {
                (0.0, available, 0.0)
            } else {
                (0.0, 0.0, 0.0)
            }
        };

        let (left_top_height, left_bottom_height, left_horizontal_divider) =
            panel_heights(self.split_ratios[0], self.visible_panels[0], self.visible_panels[1]);
        let (right_top_height, right_bottom_height, right_horizontal_divider) =
            panel_heights(self.split_ratios[1], self.visible_panels[2], self.visible_panels[3]);

        let vertical_splitter_color = if self.is_resizing_columns {
            colors.focus
        } else {
            colors.splitter
        };
        let outer_splitter_color = if self.is_resizing_outer_splitter {
            colors.focus
        } else {
            colors.splitter
        };
        let root = div()
            .flex()
            .flex_col()
            .w(px(window_width))
            .h(px(window_height))
            .bg(rgb(colors.window_background));

        let root = if self.is_resizing_columns || self.is_resizing_outer_splitter {
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
                    if this.is_resizing_outer_splitter {
                        let actual_window_width =
                            window.viewport_size().width.as_f32().max(1.0);
                        let max_total_width =
                            (actual_window_width - hit_zone * 2.0).max(min_width * 2.0);
                        let right_width = this.resize_start_column_widths[1]
                            .clamp(min_width, max_total_width - min_width);
                        let max_left_width = (max_total_width - right_width).max(min_width);
                        let delta = event.position.x.as_f32() - this.resize_start_x;
                        let left_width = (this.resize_start_column_widths[0] - delta)
                            .clamp(min_width, max_left_width);

                        this.column_widths = [left_width, right_width];
                        cx.notify();
                    } else if this.is_resizing_columns {
                        let actual_window_width =
                            window.viewport_size().width.as_f32().max(1.0);
                        let max_total_width =
                            (actual_window_width - hit_zone * 2.0).max(min_width * 2.0);
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
                        let actual_window_height = (window
                            .viewport_size()
                            .height
                            .as_f32()
                            .max(1.0)
                            - menu_area_height)
                            .max(1.0);
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
                    if this.is_resizing_columns
                        || this.is_resizing_outer_splitter
                        || this.is_resizing_height
                    {
                        this.is_resizing_columns = false;
                        this.is_resizing_outer_splitter = false;
                        this.is_resizing_height = false;
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w(px(window_width))
                    .h(px(menu_bar_height))
                    .flex_shrink_0()
                    .bg(rgb(colors.dock_background))
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(menu_bar_height))
                            .p(px(8.0))
                            .text_color(rgb(colors.primary_text))
                            .bg(rgb(if self.windows_menu_open {
                                colors.active_tab
                            } else {
                                colors.dock_background
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.windows_menu_open = !this.windows_menu_open;
                                cx.notify();
                            }))
                            .child(if self.windows_menu_open {
                                "Windows ▴"
                            } else {
                                "Windows ▾"
                            }),
                    )
                    .child(div().flex_1()),
            );

        let root = if self.windows_menu_open {
            root.child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(window_width))
                    .p(px(8.0))
                    .gap_1()
                    .bg(rgb(colors.group_background))
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(render_panel_menu_item(
                        0,
                        "Layers",
                        self.visible_panels[0],
                        self.theme,
                        cx,
                    ))
                    .child(render_panel_menu_item(
                        1,
                        "Color",
                        self.visible_panels[1],
                        self.theme,
                        cx,
                    ))
                    .child(render_panel_menu_item(
                        2,
                        "Properties",
                        self.visible_panels[2],
                        self.theme,
                        cx,
                    ))
                    .child(render_panel_menu_item(
                        3,
                        "History",
                        self.visible_panels[3],
                        self.theme,
                        cx,
                    )),
            )
        } else {
            root
        };

        root.child(
            div()
                .flex()
                .flex_row()
                .w(px(window_width))
                .h(px(workspace_height))
                .flex_shrink_0()
                .bg(rgb(colors.window_background))
                .child(
                    div()
                        .flex_1()
                        .h(px(workspace_height))
                        .bg(rgb(colors.window_background)),
                )
                // Outer splitter: resize only the left column, independently of the center splitter.
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(divider_width))
                        .h(px(workspace_height))
                        .flex_shrink_0()
                        .bg(rgb(colors.window_background))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                                this.is_resizing_columns = false;
                                this.is_resizing_outer_splitter = true;
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
                                .h(px(workspace_height))
                                .bg(rgb(outer_splitter_color)),
                        ),
                )
                .child(render_column(
                    0,
                    left_column_width,
                    left_top_height,
                    left_bottom_height,
                    self.visible_panels[0],
                    self.visible_panels[1],
                    self.theme,
                    self.active_tabs[0],
                    self.lower_active_tabs[0],
                    self.split_ratios[0],
                    left_horizontal_divider,
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
                        .h(px(workspace_height))
                        .flex_shrink_0()
                        .bg(rgb(colors.window_background))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                                this.column_widths = [left_column_width, right_column_width];
                                this.is_resizing_columns = true;
                                this.is_resizing_outer_splitter = false;
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
                                .h(px(workspace_height))
                                .bg(rgb(vertical_splitter_color)),
                        ),
                )
                .child(render_column(
                    1,
                    right_column_width,
                    right_top_height,
                    right_bottom_height,
                    self.visible_panels[2],
                    self.visible_panels[3],
                    self.theme,
                    self.active_tabs[1],
                    self.lower_active_tabs[1],
                    self.split_ratios[1],
                    right_horizontal_divider,
                    self.is_resizing_height && self.resizing_height_column == 1,
                    cx,
                )),
        )
    }
}

fn render_panel_menu_item(
    index: usize,
    label: &'static str,
    checked: bool,
    theme: Theme,
    cx: &mut Context<MainWindow>,
) -> impl IntoElement {
    let colors = theme.colors;
    let id = match index {
        0 => "windows-menu-left-upper",
        1 => "windows-menu-left-lower",
        2 => "windows-menu-right-upper",
        _ => "windows-menu-right-lower",
    };

    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(28.0))
        .p(px(4.0))
        .text_color(rgb(colors.primary_text))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.visible_panels[index] = !this.visible_panels[index];
            cx.notify();
        }))
        .child(if checked { "☑" } else { "☐" })
        .child(label)
}

fn render_column(
    column_index: usize,
    column_width: f32,
    top_panel_height: f32,
    bottom_panel_height: f32,
    show_upper_panel: bool,
    show_lower_panel: bool,
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

    let header = div()
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
        .child(control("x"));

    let mut column = div()
        .flex()
        .flex_col()
        .w(px(column_width))
        .min_w(px(metrics.min_column_width))
        .h(px(
            top_panel_height
                + horizontal_splitter_height
                + bottom_panel_height
                + metrics.column_header_height,
        ))
        .flex_shrink_0()
        .bg(rgb(colors.window_background))
        .child(header);

    if show_upper_panel {
        column = column.child(
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
                        .bg(rgb(colors.canvas_background))
                        .child(render_panel_content(
                            column_index,
                            false,
                            active_tab,
                            theme,
                        )),
                ),
        );
    }

    if show_upper_panel && show_lower_panel {
        column = column.child(
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
                        this.is_resizing_outer_splitter = false;
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
        );
    }

    if show_lower_panel {
        column = column.child(
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
                        .bg(rgb(colors.canvas_background))
                        .child(render_panel_content(
                            column_index,
                            true,
                            lower_active_tab,
                            theme,
                        )),
                ),
        );
    }

    column
}

fn render_panel_content(
    column_index: usize,
    is_lower: bool,
    active_tab: usize,
    theme: Theme,
) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let panel_name = match (column_index, is_lower) {
        (0, false) => "Layers",
        (0, true) => "Color",
        (1, false) => "Properties",
        _ => "History",
    };
    let tab_name = if active_tab == 0 {
        format!("{panel_name} — Вкладка 1")
    } else {
        format!("{panel_name} — Вкладка 2")
    };
    let details = if active_tab == 0 {
        "Здесь отображается содержимое первой вкладки."
    } else {
        "Здесь отображается содержимое второй вкладки."
    };

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p(px(metrics.content_padding))
        .text_color(rgb(colors.primary_text))
        .child(
            div()
                .text_size(px(theme.typography.title_size))
                .child(tab_name),
        )
        .child(
            div()
                .text_color(rgb(colors.muted_text))
                .child(details),
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
