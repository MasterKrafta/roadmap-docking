//! Native window for the independent dock prototype.

use gpui::{Context, IntoElement, Render, Window};
use ui::Theme;

use super::{model::DockLayout, view};

/// Owns the prototype layout and renders it in a regular GPUI window.
pub(super) struct NextDockWindow {
    theme: Theme,
    layout: DockLayout,
}

impl NextDockWindow {
    pub(super) fn new(theme: Theme) -> Self {
        Self {
            theme,
            layout: DockLayout::initial(),
        }
    }
}

impl Render for NextDockWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = viewport.width.as_f32().max(1.0);
        let height = viewport.height.as_f32().max(1.0);

        view::render(&self.layout, width, height, &self.theme)
    }
}
