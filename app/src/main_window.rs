mod tabs_panel;

use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use tabs_panel::TabsPanel;
use ui::Theme;

/// Minimal application window.
///
/// Dock panels, columns, floating windows, and pointer gestures will be rebuilt
/// from this small root instead of carrying the previous UI implementation forward.
pub(crate) struct MainWindow {
    theme: Theme,
    tabs_panel: TabsPanel,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        Ok(Self {
            theme,
            tabs_panel: TabsPanel::new(theme),
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
            .child(self.tabs_panel.render(cx, height))
    }
}
