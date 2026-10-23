use gpui::{Context, Div, ParentElement, Styled, div, px, rgb};
use super::tabs_panel::TabsPanel;
use super::MainWindow;
use ui::Theme;

/// A column consists of a header and a vertical stack of tab groups.
pub(crate) struct Column {
    theme: Theme,
    groups: Vec<TabsPanel>,
}

impl Column {
    pub(crate) fn new(theme: Theme) -> Self {
        Self {
            theme,
            groups: vec![TabsPanel::new(theme)],
        }
    }

    /// Updates the active tab in one group, ignoring an invalid group index.
    pub(crate) fn set_active_tab_id(&mut self, group_index: usize, tab_id: String) {
        if let Some(group) = self.groups.get_mut(group_index) {
            group.set_active_tab_id(tab_id);
        }
    }

    pub(crate) fn render(&self, cx: &mut Context<MainWindow>, height: f32) -> Div {
        let metrics = self.theme.metrics;
        let colors = self.theme.colors;
        let width = metrics.default_column_width.max(1.0);
        let height = height.max(1.0);
        let header_height = metrics.column_header_height.min(height).max(1.0);
        let group_count = self.groups.len();

        let button_size = metrics.button_size.min(width / 2.0).max(1.0);
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .w(px(width))
            .h(px(header_height))
            .bg(rgb(colors.toolbar_background))
            .border_1()
            .border_color(rgb(colors.border))
            // The empty header area is reserved for future drag handling.
            .child(div().flex_1().h(px(header_height)))
            .child(header_button("»", button_size, self.theme))
            .child(header_button("×", button_size, self.theme));

        let available_height = (height - header_height).max(0.0);
        let splitter_height = if group_count > 1 {
            metrics
                .splitter_thickness
                .max(0.0)
                .min(available_height / (group_count - 1) as f32)
        } else {
            0.0
        };
        let group_height = if group_count == 0 {
            0.0
        } else {
            (available_height - splitter_height * group_count.saturating_sub(1) as f32)
                .max(0.0)
                / group_count as f32
        };

        let mut column = div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.group_background))
            .child(header);

        for (index, group) in self.groups.iter().enumerate() {
            if index > 0 {
                column = column.child(
                    div()
                        .w(px(width))
                        .h(px(splitter_height))
                        .bg(rgb(colors.splitter)),
                );
            }

            column = column.child(group.render(cx, group_height, index));
        }

        column
    }
}

fn header_button(label: &'static str, size: f32, theme: Theme) -> Div {
    let colors = theme.colors;

    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(size))
        .h(px(size))
        .bg(rgb(colors.toolbar_background))
        .text_color(rgb(colors.primary_text))
        .border_1()
        .border_color(rgb(colors.border))
        .child(label)
}
