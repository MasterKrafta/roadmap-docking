use gpui::{Context, Div, ParentElement, Styled, div, px, rgb};
#[path = "column.rs"]
mod column;

use self::column::Column;
use super::MainWindow;
use ui::Theme;

/// A horizontal row of columns separated by vertical splitters.
pub(crate) struct GroupColumn {
    theme: Theme,
    columns: Vec<Column>,
}

impl GroupColumn {
    pub(crate) fn new(theme: Theme) -> Self {
        let mut group = Self {
            theme,
            columns: Vec::new(),
        };
        group.add_column();
        group
    }

    /// Adds another column until the layout reaches its configured limit.
    pub(crate) fn add_column(&mut self) {
        if self.columns.len() < self.theme.metrics.max_columns.max(1) {
            self.columns.push(Column::new(self.theme));
        }
    }

    /// Routes a tab selection to the column and group that owns it.
    pub(crate) fn set_active_tab_id(
        &mut self,
        column_index: usize,
        group_index: usize,
        tab_id: String,
    ) {
        if let Some(column) = self.columns.get_mut(column_index) {
            column.set_active_tab_id(group_index, tab_id);
        }
    }

    pub(crate) fn render(
        &self,
        cx: &mut Context<MainWindow>,
        width: f32,
        height: f32,
    ) -> Div {
        let metrics = self.theme.metrics;
        let colors = self.theme.colors;
        let width = width.max(1.0);
        let height = height.max(1.0);
        let column_count = self.columns.len();
        let splitter_width = if column_count > 1 {
            metrics.splitter_thickness.max(0.0)
        } else {
            0.0
        };

        let mut row = div()
            .flex()
            .flex_row()
            .w(px(width))
            .h(px(height))
            .bg(rgb(colors.canvas_background));

        for (index, column) in self.columns.iter().enumerate() {
            if index > 0 {
                row = row.child(
                    div()
                        .w(px(splitter_width))
                        .h(px(height))
                        .bg(rgb(colors.splitter)),
                );
            }

            row = row.child(column.render(cx, height, index));
        }

        row
    }
}
