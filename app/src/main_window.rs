#[allow(dead_code)]
mod column_tabs_group;
mod tabs_group;

use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};

/// Root application window.
pub(crate) struct MainWindow {
    theme: ui::Theme,
    active_tab: usize,
}

impl MainWindow {
    pub(crate) fn new(theme: ui::Theme) -> Result<Self, String> {
        Ok(Self {
            theme,
            active_tab: 0,
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);

        let panel_width = self.theme.metrics.default_column_width.max(1.0).min(width);

        div()
            .flex()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.canvas_background))
            .child(tabs_group::render(
                self.theme,
                self.active_tab,
                panel_width,
                height,
                cx.listener(|this, _, _, cx| {
                    this.active_tab = 0;
                    cx.notify();
                }),
                cx.listener(|this, _, _, cx| {
                    this.active_tab = 1;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .flex_1()
                    .h(px(height))
                    .bg(rgb(self.theme.colors.canvas_background)),
            )
    }
}
