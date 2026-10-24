use dock_core::{Point, Rect, Size};
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use ui::Theme;

use crate::dock::DockController;
use crate::{DockSurface, NoopHost};

/// Temporary picture-only host window used while the new surface architecture is validated.
///
/// Interaction handling and the command controller arrive in the following roadmap stages.
pub(crate) struct MainWindow {
    theme: Theme,
    dock: DockController,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        Ok(Self {
            dock: DockController::new_initial(&theme)?,
            theme,
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let menu_height = self.theme.metrics.column_header_height.max(1.0);
        let content_height = (height - menu_height).max(1.0);
        let viewport = Rect::new(Point::ZERO, Size::new(width, content_height));

        let frame = match self.dock.frame(viewport) {
            Ok(frame) => frame,
            Err(error) => {
                return div()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.window_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("Could not compute dock geometry: {error:?}"));
            }
        };
        for overflow in &frame.rects().vertical_overflow {
            self.dock
                .scroll_handles_mut()
                .entry(overflow.owner)
                .or_insert_with(gpui::ScrollHandle::new);
        }

        let mut host = NoopHost;
        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(width))
                    .h(px(menu_height))
                    .bg(rgb(self.theme.colors.toolbar_background))
                    .border_1()
                    .border_color(rgb(self.theme.colors.border))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child("Dockers ▾"),
            )
            .child(
                DockSurface::new(
                    self.dock.layout(),
                    &frame,
                    self.dock.scroll_handles(),
                    &self.theme,
                    None,
                )
                .build(&mut host),
            )
    }
}
