mod dock_controller;

use dock_controller::DockController;
use gpui::{Context, IntoElement, Render, Window};

/// Root application window. The actual docking model and GPUI integration live in DockController.
pub(crate) struct MainWindow {
    dock: DockController,
}

impl MainWindow {
    pub(crate) fn new(theme: ui::Theme) -> Result<Self, String> {
        Ok(Self {
            dock: DockController::new(theme)?,
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        self.dock
            .render(cx, viewport.width.as_f32(), viewport.height.as_f32())
    }
}
