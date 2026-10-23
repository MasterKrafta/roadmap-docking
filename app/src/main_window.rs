mod group_column;

use group_column::GroupColumn;
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use ui::Theme;

/// Root window for the rebuilt docking layout.
pub(crate) struct MainWindow {
    theme: Theme,
    group_column: GroupColumn,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        Ok(Self {
            theme,
            group_column: GroupColumn::new(theme),
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();

        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);

        div()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.canvas_background))
            .child(self.group_column.render(cx, width, height))
    }
}
