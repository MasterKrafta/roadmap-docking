use dock_core::{DragHandle, DragPayload, FloatingGroupId, Point, Rect, Size};
use gpui::{
    App, AppContext, Bounds, Context, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels,
    TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use ui::Theme;

use crate::FrameGeometry;

use super::MainWindow;

/// Immutable render snapshot shared by a native floating-window view.
pub(crate) struct FloatingWindowSnapshot {
    pub(crate) layout: dock_core::Layout,
    pub(crate) frame: FrameGeometry,
    pub(crate) theme: Theme,
    pub(crate) area: dock_core::DockAreaId,
    pub(crate) bounds: Rect,
}

struct FloatingWindowSpec {
    group: FloatingGroupId,
    title: String,
    bounds: Rect,
    native_bounds: Bounds<Pixels>,
}

impl MainWindow {
    pub(crate) fn floating_window_snapshot(
        &self,
        group: FloatingGroupId,
    ) -> Option<FloatingWindowSnapshot> {
        let viewport = self.last_dock_viewport?;
        let layout = self.dock.layout().clone();
        let metrics = *self.dock.metrics();
        let floating_bounds = layout.floating_group(group)?.bounds;
        let area = layout.floating_group(group)?.area;
        // Use a geometry viewport large enough to contain the actual native window.
        // This avoids clamping its local panel geometry to the main window's viewport.
        let right = viewport.checked_right().ok()?.max(floating_bounds.checked_right().ok()?);
        let bottom = viewport
            .checked_bottom()
            .ok()?
            .max(floating_bounds.checked_bottom().ok()?);
        let origin = Point::new(
            viewport.origin.x.min(floating_bounds.origin.x),
            viewport.origin.y.min(floating_bounds.origin.y),
        );
        let geometry_viewport = Rect::new(
            origin,
            Size::new(right - origin.x, bottom - origin.y),
        );
        let frame = FrameGeometry::compute(&layout, geometry_viewport, &metrics).ok()?;
        let bounds = frame
            .rects()
            .floating_groups
            .iter()
            .find(|candidate| candidate.group == group)?
            .rect;

        Some(FloatingWindowSnapshot {
            layout,
            frame,
            theme: self.theme,
            area,
            bounds,
        })
    }

    fn floating_window_specs(&self) -> Vec<FloatingWindowSpec> {
        let (Some(viewport), Some(parent_bounds), Some(parent_viewport)) = (
            self.last_dock_viewport,
            self.last_window_bounds,
            self.last_viewport_size,
        ) else {
            return Vec::new();
        };
        let Ok(_frame) = FrameGeometry::compute(self.dock.layout(), viewport, self.dock.metrics())
        else {
            return Vec::new();
        };

        let chrome_width =
            (parent_bounds.size.width.as_f32() - parent_viewport.width.as_f32()).max(0.0);
        let chrome_height =
            (parent_bounds.size.height.as_f32() - parent_viewport.height.as_f32()).max(0.0);
        let surface_top = self.last_surface_top;
        let layout = self.dock.layout();
        let mut specs = Vec::new();

        for (group, _) in layout.floating_groups_in_paint_order() {
            let Some(floating) = layout.floating_group(group) else {
                continue;
            };
            // Native bounds follow the model, not the frame's viewport-clamped
            // rectangle: a floating window must be allowed to leave the main window.
            let bounds = floating.bounds;
            let title = layout
                .floating_group(group)
                .and_then(|floating| layout.dock_area(floating.area))
                .and_then(|area| crate::tree_query::first_panel_in_subtree(layout, area.root))
                .and_then(|panel| layout.panel_instance(panel))
                .map(|instance| crate::panels::panel_title(instance).to_owned())
                .unwrap_or_else(|| "Floating panel".to_owned());

            let native_bounds = Bounds::new(
                point(
                    px(parent_bounds.origin.x.as_f32() + bounds.origin.x),
                    px(
                        parent_bounds.origin.y.as_f32()
                            + chrome_height
                            + surface_top
                            + bounds.origin.y,
                    ),
                ),
                size(
                    px(bounds.size.width + chrome_width),
                    px(bounds.size.height + chrome_height),
                ),
            );
            specs.push(FloatingWindowSpec {
                group,
                title,
                bounds,
                native_bounds,
            });
        }

        specs
    }

    /// Syncs an OS window move/resize back into the canonical layout without an undo step.
    pub(crate) fn sync_floating_native_bounds(
        &mut self,
        group: FloatingGroupId,
        native_bounds: Bounds<Pixels>,
        viewport_size: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<Rect> {
        let parent_bounds = self.last_window_bounds?;
        let parent_viewport = self.last_viewport_size?;
        let parent_chrome_height =
            (parent_bounds.size.height.as_f32() - parent_viewport.height.as_f32()).max(0.0);
        let origin = Point::new(
            native_bounds.origin.x.as_f32() - parent_bounds.origin.x.as_f32(),
            native_bounds.origin.y.as_f32()
                - parent_bounds.origin.y.as_f32()
                - parent_chrome_height
                - self.last_surface_top,
        );
        let bounds = Rect::new(
            origin,
            Size::new(viewport_size.width.as_f32(), viewport_size.height.as_f32()),
        );
        bounds.validate_non_empty().ok()?;

        let current_bounds = self.dock.layout().floating_group(group)?.bounds;
        if floating_bounds_match(current_bounds, bounds) {
            return Some(bounds);
        }
        match self.dock.sync_floating_bounds(group, bounds) {
            Ok(()) => {
                self.schedule_workspace_save(cx);
                cx.notify();
                Some(bounds)
            }
            Err(error) => {
                tracing::debug!(
                    group = ?group,
                    error = %error,
                    "native floating window geometry was rejected"
                );
                None
            }
        }
    }

    /// Starts a docking drag initiated in a separate native floating window.
    ///
    /// Pointer coordinates are translated into the main dock's local surface space,
    /// so the existing controller can prepare and validate drop zones unchanged.
    pub(crate) fn begin_external_drag(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        source_bounds: Bounds<Pixels>,
        source_viewport: gpui::Size<Pixels>,
        position: gpui::Point<Pixels>,
    ) {
        let Some(pointer) = self.external_surface_pointer(
            source_bounds,
            source_viewport,
            position,
        ) else {
            return;
        };
        if let Err(error) = self.dock.begin_external_drag(payload, handle, pointer) {
            tracing::debug!(error = ?error, "could not record cross-window dock drag");
        }
    }

    /// Updates the shared docking gesture from pointer events delivered to a floating window.
    pub(crate) fn handle_external_surface_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        source_bounds: Bounds<Pixels>,
        source_viewport: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.dock.resize_session().is_some() {
            return;
        }
        let Some(pointer) = self.external_surface_pointer(
            source_bounds,
            source_viewport,
            event.position,
        ) else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            if self.dock.cancel_drag() {
                cx.notify();
            }
            return;
        }
        // While the pointer is still over the source native window, don't let the
        // hidden main-surface geometry beneath it become a drop target.
        if native_window_contains_pointer(source_bounds, source_viewport, event.position) {
            return;
        }

        let Some(viewport) = self.last_dock_viewport else {
            self.dock.cancel_drag();
            cx.notify();
            return;
        };
        let frame = if self.dock.drag_needs_start(pointer) {
            match self.dock.frame(viewport) {
                Ok(frame) => Some(frame),
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        "could not compute geometry to start cross-window dock drag"
                    );
                    self.dock.cancel_drag();
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };

        match self.dock.update_drag(pointer, viewport, frame.as_ref()) {
            Ok(changed) => {
                if changed {
                    cx.notify();
                }
            }
            Err(error) => {
                tracing::warn!(error = ?error, "cross-window dock drag update failed");
                self.dock.cancel_drag();
                cx.notify();
            }
        }
    }

    /// Completes a cross-window docking gesture at the pointer's current screen position.
    pub(crate) fn handle_external_surface_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        source_bounds: Bounds<Pixels>,
        source_viewport: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.dock.resize_session().is_some() {
            return;
        }
        let Some(pointer) = self.external_surface_pointer(
            source_bounds,
            source_viewport,
            event.position,
        ) else {
            self.dock.cancel_drag();
            cx.notify();
            return;
        };
        if native_window_contains_pointer(source_bounds, source_viewport, event.position) {
            if self.dock.cancel_drag() {
                cx.notify();
            }
            return;
        }
        let Some(viewport) = self.last_dock_viewport else {
            self.dock.cancel_drag();
            cx.notify();
            return;
        };

        let had_overlay = self.dock.drag_overlay().is_some();
        let mut changed = had_overlay;
        match self.dock.finish_drag(pointer, viewport) {
            Ok(Some(request)) => {
                changed = true;
                match self.dock.commit_drop_request(request, viewport) {
                    Ok(action) => tracing::debug!(
                        action = ?action,
                        "cross-window dock drag committed"
                    ),
                    Err(error) => tracing::warn!(
                        error = %error,
                        "cross-window dock drop was rejected"
                    ),
                }
            }
            Ok(None) => {}
            Err(error) => {
                changed = true;
                tracing::warn!(error = ?error, "could not finish cross-window dock drag");
            }
        }
        self.schedule_workspace_save(cx);
        if changed {
            cx.notify();
        }
    }

    fn external_surface_pointer(
        &self,
        source_bounds: Bounds<Pixels>,
        source_viewport: gpui::Size<Pixels>,
        position: gpui::Point<Pixels>,
    ) -> Option<Point> {
        let main_bounds = self.last_window_bounds?;
        let main_viewport = self.last_viewport_size?;
        let pointer = native_pointer_in_dock_surface(
            source_bounds,
            source_viewport,
            position,
            main_bounds,
            main_viewport,
            self.last_surface_top,
        );
        pointer.is_finite().then_some(pointer)
    }
}

pub(super) fn synchronize_floating_windows(
    owner: gpui::WeakEntity<MainWindow>,
    app: &mut App,
) {
    let Some(owner_entity) = owner.upgrade() else {
        return;
    };
    let (specs, existing) = owner_entity.update(app, |main, _cx| {
        (
            main.floating_window_specs(),
            main.floating_windows
                .iter()
                .map(|(group, handle)| (*group, *handle))
                .collect::<Vec<_>>(),
        )
    });

    for (group, handle) in existing {
        let target_bounds = specs
            .iter()
            .find(|spec| spec.group == group)
            .map(|spec| spec.bounds);
        let current_bounds = handle.read(app).ok().map(|view| view.layout_bounds());
        let keep_window = target_bounds
            .zip(current_bounds)
            .is_some_and(|(target, current)| floating_bounds_match(target, current));
        if keep_window {
            continue;
        }

        if let Err(error) = handle.update(app, |_, window, _cx| window.remove_window()) {
            tracing::debug!(
                group = ?group,
                error = %error,
                "could not retire stale native floating window"
            );
        }
        owner_entity.update(app, |main, cx| {
            if main.floating_windows.remove(&group).is_some() {
                cx.notify();
            }
        });
    }

    for spec in specs {
        if owner_entity
            .read(app)
            .floating_windows
            .contains_key(&spec.group)
        {
            continue;
        }

        let group = spec.group;
        let initial_bounds = spec.bounds;
        let title = spec.title.clone();
        let native_bounds = spec.native_bounds;
        let owner_for_view = owner.clone();
        let owner_for_close = owner.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(native_bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(180.0), px(120.0))),
            ..Default::default()
        };
        match app.open_window(options, move |window, cx| {
            let view = cx.new(|_| {
                crate::floating_window::FloatingWindow::new(
                    owner_for_view.clone(),
                    group,
                    initial_bounds,
                )
            });
            view.update(cx, |_, view_cx| {
                window.on_window_should_close(view_cx, move |_window, app| {
                    if let Some(owner) = owner_for_close.upgrade() {
                        owner.update(app, |main, cx| {
                            main.close_floating_window(group, cx);
                            cx.notify();
                        });
                    }
                    true
                });
            });
            view
        }) {
            Ok(handle) => {
                let still_exists = owner_entity
                    .read(app)
                    .dock
                    .layout()
                    .floating_group(group)
                    .is_some();
                if still_exists {
                    owner_entity.update(app, |main, cx| {
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            main.floating_windows.entry(group)
                        {
                            entry.insert(handle);
                            cx.notify();
                        }
                    });
                } else if let Err(error) =
                    handle.update(app, |_, window, _cx| window.remove_window())
                {
                    tracing::debug!(
                        group = ?group,
                        error = %error,
                        "could not close native window for a removed floating group"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    group = ?group,
                    error = %error,
                    "could not open native floating window"
                );
            }
        }
    }
}

fn native_window_contains_pointer(
    window_bounds: Bounds<Pixels>,
    viewport: gpui::Size<Pixels>,
    position: gpui::Point<Pixels>,
) -> bool {
    let chrome_width =
        (window_bounds.size.width.as_f32() - viewport.width.as_f32()).max(0.0);
    let chrome_height =
        (window_bounds.size.height.as_f32() - viewport.height.as_f32()).max(0.0);
    let screen_x = window_bounds.origin.x.as_f32() + chrome_width + position.x.as_f32();
    let screen_y = window_bounds.origin.y.as_f32() + chrome_height + position.y.as_f32();
    let right = window_bounds.origin.x.as_f32() + window_bounds.size.width.as_f32();
    let bottom = window_bounds.origin.y.as_f32() + window_bounds.size.height.as_f32();

    screen_x >= window_bounds.origin.x.as_f32()
        && screen_x < right
        && screen_y >= window_bounds.origin.y.as_f32()
        && screen_y < bottom
}

fn native_pointer_in_dock_surface(
    source_bounds: Bounds<Pixels>,
    source_viewport: gpui::Size<Pixels>,
    position: gpui::Point<Pixels>,
    main_bounds: Bounds<Pixels>,
    main_viewport: gpui::Size<Pixels>,
    surface_top: f32,
) -> Point {
    let source_chrome_width =
        (source_bounds.size.width.as_f32() - source_viewport.width.as_f32()).max(0.0);
    let source_chrome_height =
        (source_bounds.size.height.as_f32() - source_viewport.height.as_f32()).max(0.0);
    let main_chrome_width =
        (main_bounds.size.width.as_f32() - main_viewport.width.as_f32()).max(0.0);
    let main_chrome_height =
        (main_bounds.size.height.as_f32() - main_viewport.height.as_f32()).max(0.0);

    let screen_x = source_bounds.origin.x.as_f32() + source_chrome_width + position.x.as_f32();
    let screen_y =
        source_bounds.origin.y.as_f32() + source_chrome_height + position.y.as_f32();
    Point::new(
        screen_x - main_bounds.origin.x.as_f32() - main_chrome_width,
        screen_y - main_bounds.origin.y.as_f32() - main_chrome_height - surface_top,
    )
}

fn floating_bounds_match(left: Rect, right: Rect) -> bool {
    const EPSILON: f32 = 1.0;
    (left.origin.x - right.origin.x).abs() < EPSILON
        && (left.origin.y - right.origin.y).abs() < EPSILON
        && (left.size.width - right.size.width).abs() < EPSILON
        && (left.size.height - right.size.height).abs() < EPSILON
}


#[cfg(test)]
mod tests {
    use super::{native_pointer_in_dock_surface, native_window_contains_pointer};
    use dock_core::Point;
    use gpui::{Bounds, point, px, size};

    #[test]
    fn external_pointer_is_translated_from_window_to_main_dock_coordinates() {
        let main_bounds = Bounds::new(
            point(px(100.0), px(200.0)),
            size(px(820.0), px(640.0)),
        );
        let source_bounds = Bounds::new(
            point(px(450.0), px(520.0)),
            size(px(420.0), px(340.0)),
        );

        let pointer = native_pointer_in_dock_surface(
            source_bounds,
            size(px(400.0), px(300.0)),
            point(px(10.0), px(20.0)),
            main_bounds,
            size(px(800.0), px(600.0)),
            24.0,
        );

        assert_eq!(pointer, Point::new(360.0, 316.0));
    }

    #[test]
    fn external_drag_does_not_target_the_main_surface_while_over_source_window() {
        let bounds = Bounds::new(
            point(px(450.0), px(520.0)),
            size(px(420.0), px(340.0)),
        );
        let viewport = size(px(400.0), px(300.0));

        assert!(native_window_contains_pointer(
            bounds,
            viewport,
            point(px(10.0), px(20.0)),
        ));
        assert!(!native_window_contains_pointer(
            bounds,
            viewport,
            point(px(450.0), px(350.0)),
        ));
    }
}
