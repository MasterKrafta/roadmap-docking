use dock_core::{Point, Rect, Size as CoreSize};
use gpui::{
    AnyView, App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Pixels, Render,
    Styled, Window, canvas, div, px, rgb,
};
use ui::Theme;

use crate::DockController;

/// Root composition for an application's docking workspace.
///
/// The application's main menu remains outside this view. The toolbar,
/// Options bar, and document canvas are supplied by the application as GPUI
/// views; their visibility follows the current Core chrome mode.
///
/// Layer order is deliberate: canvas, main dock surfaces, flyout, then
/// floating groups in ascending Core z-order. DropOverlay, DragGhost, and
/// context-menu layers are added after the corresponding components land.
pub struct DockHost {
    controller: Entity<DockController>,
    theme: Theme,
    toolbar: Option<AnyView>,
    options_bar: Option<AnyView>,
    canvas: Option<AnyView>,
}

impl DockHost {
    /// Starts a host around the application's single DockController entity.
    pub fn new(controller: Entity<DockController>, theme: Theme) -> Self {
        Self {
            controller,
            theme,
            toolbar: None,
            options_bar: None,
            canvas: None,
        }
    }

    /// Supplies the application's toolbar slot.
    ///
    /// The slot is rendered only when Core reports
    /// `ChromeMode::toolbar_visible()`.
    pub fn toolbar(mut self, view: impl Into<AnyView>) -> Self {
        self.toolbar = Some(view.into());
        self
    }

    /// Supplies the application's Options bar slot.
    ///
    /// The slot is rendered only when Core reports
    /// `ChromeMode::options_bar_visible()`.
    pub fn options_bar(mut self, view: impl Into<AnyView>) -> Self {
        self.options_bar = Some(view.into());
        self
    }

    /// Supplies the user-owned document canvas slot.
    ///
    /// When omitted, the canvas region remains a neutral theme surface.
    pub fn canvas(mut self, view: impl Into<AnyView>) -> Self {
        self.canvas = Some(view.into());
        self
    }

    /// Returns the controller entity used by this host.
    pub fn controller(&self) -> &Entity<DockController> {
        &self.controller
    }

    /// Returns the theme currently used by the host and its base surfaces.
    pub fn theme(&self) -> Theme {
        self.theme
    }
}

impl Render for DockHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Subscribe to controller changes so layout/chrome commands invalidate this host.
        let chrome = self
            .controller
            .read(cx, |controller| controller.layout().chrome());

        // The controller owns geometry caching and the Canvas-bounds update.
        // DockHost consumes only the resulting immutable snapshot.
        let rects = self.controller.update(cx, |controller, _| {
            controller.geometry().cloned()
        });
        let rects = match rects {
            Ok(rects) => Some(rects),
            Err(error) => {
                tracing::warn!(%error, "dock host could not obtain layout geometry");
                None
            }
        };

        let theme = self.theme;
        let controller_for_measurement = self.controller.clone();
        let measurement = canvas(
            move |bounds, _window, app| {
                let viewport = viewport_from_bounds(bounds);
                if viewport.validate_non_empty().is_err() {
                    return;
                }

                controller_for_measurement.update(app, |controller, controller_cx| {
                    match controller.set_viewport(viewport) {
                        Ok(true) => {
                            // geometry() performs a fresh valid calculation and updates
                            // Layout::canvas_bounds only when the measured Canvas rect changed.
                            if let Err(error) = controller.geometry() {
                                tracing::warn!(%error, "dock host geometry update failed");
                            }
                            controller_cx.notify();
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "dock host rejected measured viewport");
                        }
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.colors.window_background));

        if chrome.toolbar_visible()
            && let Some(toolbar) = self.toolbar.clone()
        {
            root = root.child(
                div()
                    .w_full()
                    .flex_none()
                    .bg(rgb(theme.colors.toolbar_background))
                    .child(toolbar),
            );
        }

        if chrome.options_bar_visible()
            && let Some(options_bar) = self.options_bar.clone()
        {
            root = root.child(
                div()
                    .w_full()
                    .flex_none()
                    .bg(rgb(theme.colors.toolbar_background))
                    .child(options_bar),
            );
        }

        let mut workspace = div()
            .relative()
            .flex_1()
            .w_full()
            .bg(rgb(theme.colors.window_background))
            .overflow_hidden()
            .child(measurement);

        // Base canvas surface prevents an uninitialized/empty workspace from
        // exposing an undefined background while the first geometry pass runs.
        if let Some(canvas_rect) = rects.as_ref().and_then(|rects| rects.canvas) {
            let mut canvas_surface = div()
                .absolute()
                .left(px(canvas_rect.origin.x))
                .top(px(canvas_rect.origin.y))
                .w(px(canvas_rect.size.width))
                .h(px(canvas_rect.size.height))
                .bg(rgb(theme.colors.canvas_background));
            if let Some(canvas_view) = self.canvas.clone() {
                canvas_surface = canvas_surface.child(
                    div().size_full().child(canvas_view),
                );
            }
            workspace = workspace.child(canvas_surface);
        } else {
            let mut canvas_surface = div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgb(theme.colors.canvas_background));
            if let Some(canvas_view) = self.canvas.clone() {
                canvas_surface = canvas_surface.child(canvas_view);
            }
            workspace = workspace.child(canvas_surface);
        }

        if chrome.dock_visible()
            && let Some(rects) = rects.as_ref()
        {
            // MainDockView/ColumnView will replace these base surfaces in §5.2.
            for column in &rects.columns {
                workspace = workspace.child(
                    div()
                        .absolute()
                        .left(px(column.rect.origin.x))
                        .top(px(column.rect.origin.y))
                        .w(px(column.rect.size.width))
                        .h(px(column.rect.size.height))
                        .bg(rgb(theme.colors.dock_background)),
                );
            }

            // Flyout is painted above the Canvas and main dock surfaces.
            if let Some(flyout) = rects.flyout {
                workspace = workspace.child(
                    div()
                        .absolute()
                        .left(px(flyout.rect.origin.x))
                        .top(px(flyout.rect.origin.y))
                        .w(px(flyout.rect.size.width))
                        .h(px(flyout.rect.size.height))
                        .bg(rgb(theme.colors.panel_surface_background)),
                );
            }

            // Later children paint above earlier ones; Core z_order is preserved.
            let mut floating_groups = rects.floating_groups.clone();
            floating_groups.sort_by_key(|group| group.z_order);
            for floating in floating_groups {
                workspace = workspace.child(
                    div()
                        .absolute()
                        .left(px(floating.rect.origin.x))
                        .top(px(floating.rect.origin.y))
                        .w(px(floating.rect.size.width))
                        .h(px(floating.rect.size.height))
                        .bg(rgb(theme.colors.panel_surface_background))
                        .border_1()
                        .border_color(rgb(theme.colors.group_frame_border)),
                );
            }
        }

        root.child(workspace)
    }
}

/// Converts GPUI's logical bounds to the host-local Core viewport.
///
/// Core geometry is expressed relative to the DockHost, so the GPUI window
/// origin is intentionally not copied into the viewport origin.
fn viewport_from_bounds(bounds: Bounds<Pixels>) -> Rect {
    Rect::new(
        Point::ZERO,
        CoreSize::new(
            bounds.size.width.as_f32(),
            bounds.size.height.as_f32(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, Point as GpuiPoint, Size as GpuiSize, px};

    use super::viewport_from_bounds;
    use dock_core::{Point, Rect, Size};

    #[test]
    fn viewport_uses_host_local_origin_and_logical_size() {
        let bounds = Bounds {
            origin: GpuiPoint::new(px(18.0), px(32.0)),
            size: GpuiSize::new(px(960.0), px(640.0)),
        };

        assert_eq!(
            viewport_from_bounds(bounds),
            Rect::new(Point::ZERO, Size::new(960.0, 640.0))
        );
    }
}
