use gpui::{
    Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
    rgb,
};
use ui::Theme;

/// Two dock columns: the left has two groups with two tabs each; the right has one group and one tab.
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
    visible_tabs: [bool; 5],
    collapsed_columns: [bool; 2],
    collapsed_group_content: [bool; 3],
    windows_menu_open: bool,
    resizing_height_column: usize,
    is_resizing_columns: bool,
    is_resizing_outer_splitter: bool,
    resizing_outer_right: bool,
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
            visible_tabs: [true, true, true, true, true],
            collapsed_columns: [false, false],
            collapsed_group_content: [false, false, false],
            windows_menu_open: false,
            resizing_height_column: 0,
            is_resizing_columns: false,
            is_resizing_outer_splitter: false,
            resizing_outer_right: false,
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
        let menu_dropdown_height = menu_item_height * 5.0 + 32.0;
        // The menu is an overlay; only the top menu bar occupies layout space.
        let menu_area_height = menu_bar_height;
        let workspace_height = (window_height - menu_bar_height).max(1.0);

        let min_column_width = metrics.min_column_width;
        let divider_width = metrics.splitter_hit_zone;
        let left_upper_group_visible = self.visible_tabs[0] || self.visible_tabs[1];
        let left_lower_group_visible = self.visible_tabs[2] || self.visible_tabs[3];
        let right_group_visible = self.visible_tabs[4];
        let left_column_visible = left_upper_group_visible || left_lower_group_visible;
        let right_column_visible = right_group_visible;
        let both_columns_visible = left_column_visible && right_column_visible;
        let any_column_visible = left_column_visible || right_column_visible;
        let center_splitter_resizable = both_columns_visible
            && !self.collapsed_columns[0]
            && !self.collapsed_columns[1];
        let outer_splitter_resizable = (left_column_visible
            && !self.collapsed_columns[0]
            && (!right_column_visible || !self.collapsed_columns[1]))
            || (!left_column_visible
                && right_column_visible
                && !self.collapsed_columns[1]);

        let collapsed_column_width = (metrics.button_size * 2.0 + 8.0).max(48.0);
        let left_min_width = if self.collapsed_columns[0] {
            collapsed_column_width
        } else {
            min_column_width
        };
        let right_min_width = if self.collapsed_columns[1] {
            collapsed_column_width
        } else {
            min_column_width
        };
        let left_base_width = if self.collapsed_columns[0] {
            collapsed_column_width
        } else {
            self.column_widths[0]
        };
        let right_base_width = if self.collapsed_columns[1] {
            collapsed_column_width
        } else {
            self.column_widths[1]
        };
        let required_pair_width = left_min_width + right_min_width;
        let max_pair_width =
            (window_width - divider_width * 2.0).max(required_pair_width);
        let (left_column_width, right_column_width) =
            match (left_column_visible, right_column_visible) {
                (true, true) => {
                    let pair_width = (left_base_width + right_base_width)
                        .min(max_pair_width)
                        .max(required_pair_width);
                    let left = left_base_width
                        .clamp(left_min_width, pair_width - right_min_width);
                    (left, pair_width - left)
                }
                (true, false) => (
                    left_base_width.clamp(left_min_width, (window_width - divider_width).max(left_min_width)),
                    0.0,
                ),
                (false, true) => (
                    0.0,
                    right_base_width.clamp(right_min_width, (window_width - divider_width).max(right_min_width)),
                ),
                (false, false) => (0.0, 0.0),
            };

        let header_height = metrics.column_header_height.min(workspace_height);
        let full_panel_height = (workspace_height - header_height).max(0.0);
        let horizontal_divider_height = metrics.splitter_hit_zone.min(full_panel_height);

        let panel_heights = |
            ratio: f32,
            show_top: bool,
            show_bottom: bool,
            top_content_collapsed: bool,
            bottom_content_collapsed: bool,
        | {
            if show_top && show_bottom {
                if top_content_collapsed || bottom_content_collapsed {
                    if top_content_collapsed && bottom_content_collapsed {
                        // Both groups retain their tab strips, but neither owns content height.
                        (metrics.tab_strip_height, metrics.tab_strip_height, 0.0)
                    } else if top_content_collapsed {
                        (
                            metrics.tab_strip_height,
                            (full_panel_height - metrics.tab_strip_height).max(0.0),
                            0.0,
                        )
                    } else {
                        (
                            (full_panel_height - metrics.tab_strip_height).max(0.0),
                            metrics.tab_strip_height,
                            0.0,
                        )
                    }
                } else {
                    let divider = horizontal_divider_height;
                    let available = (full_panel_height - divider).max(0.0);
                    let min_height = metrics.min_group_height.min(available / 2.0);
                    let max_top_height = (available - min_height).max(min_height);
                    let top = (available * ratio).clamp(min_height, max_top_height);
                    (top, (available - top).max(0.0), divider)
                }
            } else if show_top {
                (
                    if top_content_collapsed {
                        metrics.tab_strip_height
                    } else {
                        full_panel_height
                    },
                    0.0,
                    0.0,
                )
            } else if show_bottom {
                (
                    0.0,
                    if bottom_content_collapsed {
                        metrics.tab_strip_height
                    } else {
                        full_panel_height
                    },
                    0.0,
                )
            } else {
                (0.0, 0.0, 0.0)
            }
        };

        let (left_top_height, left_bottom_height, left_horizontal_divider) = panel_heights(
            self.split_ratios[0],
            left_upper_group_visible,
            left_lower_group_visible,
            self.collapsed_group_content[0],
            self.collapsed_group_content[1],
        );
        let (right_top_height, right_bottom_height, right_horizontal_divider) =
            panel_heights(
                self.split_ratios[1],
                right_group_visible,
                false,
                self.collapsed_group_content[2],
                false,
            );

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .w(px(window_width))
            .h(px(window_height))
            .bg(rgb(colors.window_background));

        if self.is_resizing_columns || self.is_resizing_outer_splitter {
            root = root.cursor(CursorStyle::ResizeLeftRight);
        } else if self.is_resizing_height {
            root = root.cursor(CursorStyle::ResizeUpDown);
        }

        root = root
            .on_mouse_move({
                let min_width = metrics.min_column_width;
                let hit_zone = metrics.splitter_hit_zone;
                let min_group_height = metrics.min_group_height;
                let column_header_height = metrics.column_header_height;
                let default_split = metrics.default_column_split_ratio;
                let both_columns_visible = both_columns_visible;

                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    if this.is_resizing_outer_splitter {
                        let actual_window_width =
                            window.viewport_size().width.as_f32().max(1.0);
                        let delta = event.position.x.as_f32() - this.resize_start_x;

                        if this.resizing_outer_right {
                            let max_width = (actual_window_width - hit_zone).max(min_width);
                            let right_width = (this.resize_start_column_widths[1] - delta)
                                .clamp(min_width, max_width);
                            this.column_widths = [this.resize_start_column_widths[0], right_width];
                        } else {
                            let max_left_width = if both_columns_visible {
                                let max_total_width =
                                    (actual_window_width - hit_zone * 2.0).max(min_width * 2.0);
                                (max_total_width - this.resize_start_column_widths[1])
                                    .max(min_width)
                            } else {
                                (actual_window_width - hit_zone).max(min_width)
                            };
                            let left_width = (this.resize_start_column_widths[0] - delta)
                                .clamp(min_width, max_left_width);
                            this.column_widths =
                                [left_width, this.resize_start_column_widths[1]];
                        }
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
                            .id("windows-menu-button")
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

        let mut dock_row = div()
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
            );

        if any_column_visible {
            let outer_right_only = !left_column_visible && right_column_visible;
            let remembered_left_width = self.column_widths[0];
            let remembered_right_width = self.column_widths[1];
            let mut outer_splitter = div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(divider_width))
                .h(px(workspace_height))
                .flex_shrink_0()
                .bg(rgb(colors.window_background))
                .child(
                    div()
                        .w(px(metrics.splitter_thickness))
                        .h(px(workspace_height))
                        .bg(rgb(colors.splitter)),
                );

            if outer_splitter_resizable {
                outer_splitter = outer_splitter
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                            this.is_resizing_columns = false;
                            this.is_resizing_outer_splitter = true;
                            this.resizing_outer_right = outer_right_only;
                            this.is_resizing_height = false;
                            this.resize_start_x = event.position.x.as_f32();
                            this.resize_start_column_widths = [
                                if outer_right_only {
                                    remembered_left_width
                                } else {
                                    left_column_width.max(min_column_width)
                                },
                                if both_columns_visible || outer_right_only {
                                    right_column_width
                                } else {
                                    remembered_right_width
                                },
                            ];
                            cx.notify();
                        }),
                    );
            }

            dock_row = dock_row.child(outer_splitter);
        }

        if left_column_visible {
            dock_row = dock_row.child(render_column(
                0,
                left_column_width,
                left_top_height,
                left_bottom_height,
                left_upper_group_visible,
                left_lower_group_visible,
                [self.visible_tabs[0], self.visible_tabs[1]],
                [self.visible_tabs[2], self.visible_tabs[3]],
                self.collapsed_columns[0],
                self.theme,
                self.active_tabs[0],
                self.lower_active_tabs[0],
                self.split_ratios[0],
                left_horizontal_divider,
                self.collapsed_group_content[0],
                self.collapsed_group_content[1],
                cx,
            ));
        }

        if both_columns_visible {
            let mut center_splitter = div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(divider_width))
                .h(px(workspace_height))
                .flex_shrink_0()
                .bg(rgb(colors.window_background))
                .child(
                    div()
                        .w(px(metrics.splitter_thickness))
                        .h(px(workspace_height))
                        .bg(rgb(colors.splitter)),
                );

            if center_splitter_resizable {
                center_splitter = center_splitter
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                            this.column_widths = [left_column_width, right_column_width];
                            this.is_resizing_columns = true;
                            this.is_resizing_outer_splitter = false;
                            this.resizing_outer_right = false;
                            this.is_resizing_height = false;
                            this.resize_start_x = event.position.x.as_f32();
                            this.resize_start_column_widths =
                                [left_column_width, right_column_width];
                            cx.notify();
                        }),
                    );
            }

            dock_row = dock_row.child(center_splitter);
        }

        if right_column_visible {
            dock_row = dock_row.child(render_column(
                1,
                right_column_width,
                right_top_height,
                right_bottom_height,
                right_group_visible,
                false,
                [self.visible_tabs[4], false],
                [false, false],
                self.collapsed_columns[1],
                self.theme,
                self.active_tabs[1],
                self.lower_active_tabs[1],
                self.split_ratios[1],
                right_horizontal_divider,
                self.collapsed_group_content[2],
                false,
                cx,
            ));
        }

        root = root.child(dock_row);

        if self.windows_menu_open {
            root = root.child(
                div()
                    .absolute()
                    .top(px(menu_bar_height))
                    .left(px(0.0))
                    .w(px(300.0))
                    .h(px(menu_dropdown_height))
                    .flex()
                    .flex_col()
                    .p(px(8.0))
                    .gap_1()
                    .bg(rgb(colors.group_background))
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(render_panel_menu_item(
                        0,
                        "Слои",
                        self.visible_tabs[0],
                        self.theme,
                        cx,
                    ))
                    .child(render_panel_menu_item(
                        1,
                        "Навигатор",
                        self.visible_tabs[1],
                        self.theme,
                        cx,
                    ))
                    .child(render_panel_menu_item(
                        2,
                        "Цвет",
                        self.visible_tabs[2],
                        self.theme,
                        cx,
                    ))
                    .child(render_panel_menu_item(
                        3,
                        "История",
                        self.visible_tabs[3],
                        self.theme,
                        cx,
                    ))
                    .child(render_panel_menu_item(
                        4,
                        "Свойства",
                        self.visible_tabs[4],
                        self.theme,
                        cx,
                    )),
            );
        }

        root
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
        0 => "windows-menu-left-upper-tab-1",
        1 => "windows-menu-left-upper-tab-2",
        2 => "windows-menu-left-lower-tab-1",
        3 => "windows-menu-left-lower-tab-2",
        _ => "windows-menu-right-tab-1",
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
            this.visible_tabs[index] = !this.visible_tabs[index];

            // Keep a visible tab selected whenever exactly one tab remains in a group.
            match index {
                0 | 1 => {
                    if this.visible_tabs[0] && !this.visible_tabs[1] {
                        this.active_tabs[0] = 0;
                    } else if !this.visible_tabs[0] && this.visible_tabs[1] {
                        this.active_tabs[0] = 1;
                    }
                }
                2 | 3 => {
                    if this.visible_tabs[2] && !this.visible_tabs[3] {
                        this.lower_active_tabs[0] = 0;
                    } else if !this.visible_tabs[2] && this.visible_tabs[3] {
                        this.lower_active_tabs[0] = 1;
                    }
                }
                _ => {}
            }

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
    upper_tabs_visible: [bool; 2],
    lower_tabs_visible: [bool; 2],
    is_collapsed: bool,
    theme: Theme,
    active_tab: usize,
    lower_active_tab: usize,
    split_ratio: f32,
    horizontal_splitter_height: f32,
    upper_content_collapsed: bool,
    lower_content_collapsed: bool,
    cx: &mut Context<MainWindow>,
) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
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

    let toggle_collapse = cx.listener(move |this, _, _, cx| {
        this.collapsed_columns[column_index] = !this.collapsed_columns[column_index];
        cx.notify();
    });
    let toggle_collapse_on_double_click =
        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
            if event.click_count >= 2 {
                this.collapsed_columns[column_index] = !this.collapsed_columns[column_index];
                cx.notify();
            }
        });
    let close_column = cx.listener(move |this, _, _, cx| {
        if column_index == 0 {
            this.visible_tabs[0] = false;
            this.visible_tabs[1] = false;
            this.visible_tabs[2] = false;
            this.visible_tabs[3] = false;
        } else {
            this.visible_tabs[4] = false;
        }
        this.collapsed_columns[column_index] = false;
        this.is_resizing_columns = false;
        this.is_resizing_outer_splitter = false;
        this.is_resizing_height = false;
        cx.notify();
    });

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(metrics.column_header_height))
        .flex_shrink_0()
        .bg(rgb(colors.group_background))
        .border_b_1()
        .border_color(rgb(colors.border))
        .child(
            div()
                .flex_1()
                .on_mouse_down(MouseButton::Left, toggle_collapse_on_double_click),
        )
        .child(
            div()
                .id(if column_index == 0 {
                    "left-column-collapse-button"
                } else {
                    "right-column-collapse-button"
                })
                .flex()
                .items_center()
                .justify_center()
                .w(px(metrics.button_size))
                .h(px(metrics.button_size))
                .text_color(rgb(colors.muted_text))
                .on_click(toggle_collapse)
                .child(if is_collapsed { "<<" } else { ">>" }),
        )
        .child(
            div()
                .id(if column_index == 0 {
                    "left-column-close-button"
                } else {
                    "right-column-close-button"
                })
                .flex()
                .items_center()
                .justify_center()
                .w(px(metrics.button_size))
                .h(px(metrics.button_size))
                .text_color(rgb(colors.muted_text))
                .on_click(close_column)
                .child("x"),
        );

    let column_height = top_panel_height
        + horizontal_splitter_height
        + bottom_panel_height
        + metrics.column_header_height;
    let mut column = div()
        .flex()
        .flex_col()
        .w(px(column_width))
        .h(px(column_height))
        .flex_shrink_0()
        .bg(rgb(colors.window_background))
        .child(header);

    if is_collapsed {
        let mut icon_rail = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .w(px(column_width))
            .h(px((column_height - metrics.column_header_height).max(0.0)))
            .flex_shrink_0()
            .bg(rgb(colors.group_background));

        if column_index == 0 {
            if upper_tabs_visible[0] {
                icon_rail = icon_rail.child(render_collapsed_tab_icon(
                    0, "▱", column_width, theme, cx,
                ));
            }
            if upper_tabs_visible[1] {
                icon_rail = icon_rail.child(render_collapsed_tab_icon(
                    1, "◇", column_width, theme, cx,
                ));
            }
            if lower_tabs_visible[0] {
                icon_rail = icon_rail.child(render_collapsed_tab_icon(
                    2, "◉", column_width, theme, cx,
                ));
            }
            if lower_tabs_visible[1] {
                icon_rail = icon_rail.child(render_collapsed_tab_icon(
                    3, "↶", column_width, theme, cx,
                ));
            }
        } else if upper_tabs_visible[0] {
            icon_rail = icon_rail.child(render_collapsed_tab_icon(
                4, "☷", column_width, theme, cx,
            ));
        }

        return column.child(icon_rail);
    }

    if show_upper_panel {
        let mut upper_group = div()
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
                upper_tabs_visible[0],
                upper_tabs_visible[1],
                cx,
            ));

        if !upper_content_collapsed {
            upper_group = upper_group.child(
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
            );
        }

        column = column.child(upper_group);
    }

    if show_upper_panel
        && show_lower_panel
        && !upper_content_collapsed
        && !lower_content_collapsed
    {
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
                        .bg(rgb(colors.splitter)),
                ),
        );
    }

    if show_lower_panel {
        let mut lower_group = div()
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
                lower_tabs_visible[0],
                lower_tabs_visible[1],
                cx,
            ));

        if !lower_content_collapsed {
            lower_group = lower_group.child(
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
            );
        }

        column = column.child(lower_group);
    }

    column
}

fn render_collapsed_tab_icon(
    tab_index: usize,
    icon: &'static str,
    column_width: f32,
    theme: Theme,
    cx: &mut Context<MainWindow>,
) -> impl IntoElement {
    let metrics = theme.metrics;
    let colors = theme.colors;
    let id = match tab_index {
        0 => "collapsed-icon-layers",
        1 => "collapsed-icon-navigator",
        2 => "collapsed-icon-color",
        3 => "collapsed-icon-history",
        _ => "collapsed-icon-properties",
    };

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(column_width))
        .h(px(metrics.button_size))
        .flex_shrink_0()
        .bg(rgb(colors.dock_background))
        .text_color(rgb(colors.primary_text))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.collapsed_columns[if tab_index == 4 { 1 } else { 0 }] = false;
            match tab_index {
                0 => this.active_tabs[0] = 0,
                1 => this.active_tabs[0] = 1,
                2 => this.lower_active_tabs[0] = 0,
                3 => this.lower_active_tabs[0] = 1,
                _ => this.active_tabs[1] = 0,
            }
            cx.notify();
        }))
        .child(icon)
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
    let tab_name = match (column_index, is_lower, active_tab) {
        (0, false, 0) => "Слои",
        (0, false, _) => "Навигатор",
        (0, true, 0) => "Цвет",
        (0, true, _) => "История",
        (1, false, _) => "Свойства",
        _ => panel_name,
    };
    let details = format!("Содержимое панели «{tab_name}».");

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
    first_tab_visible: bool,
    second_tab_visible: bool,
    cx: &mut Context<MainWindow>,
) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let tab_width =
        metrics.column_tab_width.min(((column_width - metrics.button_size).max(0.0)) / 2.0);
    let (first_tab_label, second_tab_label) = match (column_index, is_lower) {
        (0, false) => ("Слои", "Навигатор"),
        (0, true) => ("Цвет", "История"),
        _ => ("Свойства", ""),
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

    let content_index = match (column_index, is_lower) {
        (0, false) => 0,
        (0, true) => 1,
        _ => 2,
    };
    let toggle_content_first =
        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
            if event.click_count >= 2 {
                this.collapsed_group_content[content_index] =
                    !this.collapsed_group_content[content_index];
                cx.notify();
            }
        });
    let toggle_content_second =
        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
            if event.click_count >= 2 {
                this.collapsed_group_content[content_index] =
                    !this.collapsed_group_content[content_index];
                cx.notify();
            }
        });
    let toggle_content_empty =
        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
            if event.click_count >= 2 {
                this.collapsed_group_content[content_index] =
                    !this.collapsed_group_content[content_index];
                cx.notify();
            }
        });

    let control = div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(metrics.button_size))
        .h(px(metrics.button_size))
        .text_color(rgb(colors.muted_text))
        .child("☰");

    let mut strip = div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(metrics.tab_strip_height))
        .flex_shrink_0()
        .bg(rgb(colors.dock_background))
        .border_b_1()
        .border_color(rgb(colors.border));

    if first_tab_visible {
        strip = strip.child(
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
                .on_mouse_down(MouseButton::Left, toggle_content_first)
                .child(first_tab_label),
        );
    }

    if second_tab_visible {
        strip = strip.child(
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
                .on_mouse_down(MouseButton::Left, toggle_content_second)
                .child(second_tab_label),
        );
    }

    strip
        .child(
            div()
                .flex_1()
                .h(px(metrics.tab_strip_height))
                .on_mouse_down(MouseButton::Left, toggle_content_empty),
        )
        .child(control)
}
